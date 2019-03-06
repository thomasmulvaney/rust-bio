// Copyright 2014-2018 Johannes Köster, Christopher Schröder, Henning Timm.
// Licensed under the MIT license (http://opensource.org/licenses/MIT)
// This file may not be copied, modified, or distributed
// except according to those terms.

//! FASTA format reading and writing.
//!
//! # Example
//!
//! This example reads FASTA files from STDIN and prints the id and sequence length of
//! of each record to STDOUT.
//!
//! ```no_run
//! use std::error::Error;
//! use std::io;
//! use bio::io::fasta;
//!
//! fn main() -> Result<(), Box<Error>> {
//!     let reader = fasta::Reader::new(io::stdin());
//!
//!     for result in reader.records() {
//!         let record = result?;
//!         println!("{} {}", record.id(), record.seq().len());
//!     }
//!     Ok(())
//! }
//! ```

use std::cmp::min;
use std::collections;
use std::convert::AsRef;
use std::fs;
use std::io;
use std::io::prelude::*;
use std::path::Path;

use csv;

use crate::utils::{Text, TextSlice};

/// Maximum size of temporary buffer used for reading indexed FASTA files.
const MAX_FASTA_BUFFER_SIZE: usize = 512;

/// Trait for FASTA readers.
pub trait FastaRead {
    fn read(&mut self, record: &mut Record) -> io::Result<()>;
}

/// A FASTA reader.
#[derive(Debug)]
pub struct Reader<R: io::Read> {
    reader: io::BufReader<R>,
    line: String,
}

impl Reader<fs::File> {
    /// Creates a FASTA reader for the given path.
    ///
    /// # Errors
    ///
    /// If there are any issues opening the file an error
    /// variant will be returned.
    ///
    /// # Example
    ///
    /// This example opens the file `C_elegans.fasta` and prints
    /// the ids of each contig.
    ///
    /// ```no_run
    /// use std::error::Error;
    /// use bio::io::fasta::Reader;
    ///
    /// fn main() -> Result<(), Box<Error>> {
    ///     // Check for errors when opening from a file
    ///     let mut reader = Reader::from_file("C_elegans.fasta")?;
    ///     for result in reader.records() {
    ///         // Check for errors when processing records
    ///         let record = result?;
    ///         println!("{}", record.id());
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub fn from_file<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        fs::File::open(path).map(Reader::new)
    }
}

impl<R: io::Read> Reader<R> {
    /// Create a new FASTA reader given an instance of `io::Read`.
    ///
    /// # Example
    ///
    /// This example builds a reader from a sequence of `bytes`.
    ///
    /// ```rust
    /// use std::error::Error;
    /// use std::io;
    /// use bio::io::fasta::Reader;
    /// fn main() -> Result<(), Box<Error>> {
    ///     const fasta_file: &'static [u8] = b">id desc
    /// AAAA
    /// ";
    ///     let reader = Reader::new(fasta_file);
    ///     Ok(())
    /// }
    /// ```
    pub fn new(reader: R) -> Self {
        Reader {
            reader: io::BufReader::new(reader),
            line: String::new(),
        }
    }

    /// Return an iterator over the records of a FASTA file.
    ///
    /// # Example
    /// ```rust
    /// use std::error::Error;
    /// use bio::io::fasta::Reader;
    ///
    /// fn main() -> Result<(), Box<Error>> {
    ///     const fasta_file: &'static [u8] = b">id desc
    /// AAAA
    /// ";
    ///     let reader = Reader::new(fasta_file);
    ///     for record in reader.records() {
    ///         // Check for errors
    ///         let record = record?;
    ///         assert_eq!(record.id(), "id");
    ///         assert_eq!(record.desc().unwrap(), "desc");
    ///         assert_eq!(record.seq().to_vec(), b"AAAA");
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub fn records(self) -> Records<R> {
        Records {
            reader: self,
            error_has_occured: false,
        }
    }
}

impl<R> FastaRead for Reader<R>
where
    R: io::Read,
{
    /// Read the next FASTA record into the given `Record`.
    /// An empty record indicates that no more records can be read.
    ///
    /// Use this method when you want to read records as fast as
    /// possible because it allows the reuse of a `Record` allocation.
    ///
    /// The [records](Reader::records) iterator provides a more ergonomic
    /// approach to accessing FASTA records.
    ///
    /// # Errors
    ///
    /// This function will return an error if the record is incomplete,
    /// syntax is violated or any form of I/O error is encountered.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::error::Error;
    /// use bio::io::fasta::{Reader, FastaRead};
    /// use bio::io::fasta::Record;
    ///
    /// fn main() -> Result<(), Box<Error>> {
    ///     const fasta_file: &'static [u8] = b">id desc
    /// AAAA
    /// ";
    ///     let mut reader = Reader::new(fasta_file);
    ///     let mut record = Record::new();
    ///
    ///     // Check for errors parsing the record
    ///     reader.read(&mut record)?;
    ///
    ///     assert_eq!(record.id(), "id");
    ///     assert_eq!(record.desc().unwrap(), "desc");
    ///     assert_eq!(record.seq().to_vec(), b"AAAA");
    ///     Ok(())
    /// }
    /// ```
    fn read(&mut self, record: &mut Record) -> io::Result<()> {
        record.clear();
        if self.line.is_empty() {
            r#try!(self.reader.read_line(&mut self.line));
            if self.line.is_empty() {
                return Ok(());
            }
        }

        if !self.line.starts_with('>') {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Expected > at record start.",
            ));
        }
        let mut header_fields = self.line[1..].trim_right().splitn(2, ' ');
        record.id = header_fields.next().map(|s| s.to_owned()).unwrap();
        record.desc = header_fields.next().map(|s| s.to_owned());
        loop {
            self.line.clear();
            r#try!(self.reader.read_line(&mut self.line));
            if self.line.is_empty() || self.line.starts_with('>') {
                break;
            }
            record.seq.push_str(self.line.trim_right());
        }

        Ok(())
    }
}

/// A FASTA index as created by SAMtools (.fai).
#[derive(Debug, Clone)]
pub struct Index {
    inner: Vec<IndexRecord>,
    name_to_rid: collections::HashMap<String, usize>,
}

impl Index {
    /// Open a FASTA index from a given `io::Read` instance.
    pub fn new<R: io::Read>(fai: R) -> csv::Result<Self> {
        let mut inner = vec![];
        let mut name_to_rid = collections::HashMap::new();

        let mut fai_reader = csv::ReaderBuilder::new()
            .delimiter(b'\t')
            .has_headers(false)
            .from_reader(fai);
        for (rid, row) in fai_reader.deserialize().enumerate() {
            let record: IndexRecord = row?;
            name_to_rid.insert(record.name.clone(), rid);
            inner.push(record);
        }
        Ok(Index { inner, name_to_rid })
    }

    /// Open a FASTA index from a given file path.
    pub fn from_file<P: AsRef<Path>>(path: &P) -> csv::Result<Self> {
        fs::File::open(path)
            .map_err(csv::Error::from)
            .and_then(Self::new)
    }

    /// Open a FASTA index given the corresponding FASTA file path.
    /// That is, for ref.fasta we expect ref.fasta.fai.
    pub fn with_fasta_file<P: AsRef<Path>>(fasta_path: &P) -> csv::Result<Self> {
        let mut fai_path = fasta_path.as_ref().as_os_str().to_owned();
        fai_path.push(".fai");

        Self::from_file(&fai_path)
    }

    /// Return a vector of sequences described in the index.
    pub fn sequences(&self) -> Vec<Sequence> {
        // sort kv pairs by rid to preserve order
        self.inner
            .iter()
            .map(|record| Sequence {
                name: record.name.clone(),
                len: record.len,
            })
            .collect()
    }
}

/// A FASTA reader with an index as created by SAMtools (.fai).
#[derive(Debug)]
pub struct IndexedReader<R: io::Read + io::Seek> {
    reader: io::BufReader<R>,
    pub index: Index,
    fetched_idx: Option<IndexRecord>,
    start: Option<u64>,
    stop: Option<u64>,
}

impl IndexedReader<fs::File> {
    /// Read from a given file path. This assumes the index ref.fasta.fai to be
    /// present for FASTA ref.fasta.
    pub fn from_file<P: AsRef<Path>>(path: &P) -> csv::Result<Self> {
        let index = Index::with_fasta_file(path)?;
        fs::File::open(path)
            .map(|f| Self::with_index(f, index))
            .map_err(csv::Error::from)
    }
}

impl<R: io::Read + io::Seek> IndexedReader<R> {
    /// Read from a FASTA and its index, both given as `io::Read`. FASTA has to
    /// be `io::Seek` in addition.
    pub fn new<I: io::Read>(fasta: R, fai: I) -> csv::Result<Self> {
        let index = r#try!(Index::new(fai));
        Ok(IndexedReader {
            reader: io::BufReader::new(fasta),
            index,
            fetched_idx: None,
            start: None,
            stop: None,
        })
    }

    /// Read from a FASTA and its index, the first given as `io::Read`, the
    /// second given as index object.
    pub fn with_index(fasta: R, index: Index) -> Self {
        IndexedReader {
            reader: io::BufReader::new(fasta),
            index,
            fetched_idx: None,
            start: None,
            stop: None,
        }
    }

    /// Fetch an interval from the sequence with the given name for reading.
    /// (stop position is exclusive).
    pub fn fetch(&mut self, seq_name: &str, start: u64, stop: u64) -> io::Result<()> {
        let idx = self.idx(seq_name)?;
        self.start = Some(start);
        self.stop = Some(stop);
        self.fetched_idx = Some(idx);
        Ok(())
    }

    /// Fetch an interval from the sequence with the given record index for reading.
    /// (stop position is exclusive).
    pub fn fetch_by_rid(&mut self, rid: usize, start: u64, stop: u64) -> io::Result<()> {
        let idx = self.idx_by_rid(rid)?;
        self.start = Some(start);
        self.stop = Some(stop);
        self.fetched_idx = Some(idx);
        Ok(())
    }

    /// Fetch the whole sequence with the given name for reading.
    pub fn fetch_all(&mut self, seq_name: &str) -> io::Result<()> {
        let idx = self.idx(seq_name)?;
        self.start = Some(0);
        self.stop = Some(idx.len);
        self.fetched_idx = Some(idx);
        Ok(())
    }

    /// Fetch the whole sequence with the given record index for reading.
    pub fn fetch_all_by_rid(&mut self, rid: usize) -> io::Result<()> {
        let idx = self.idx_by_rid(rid)?;
        self.start = Some(0);
        self.stop = Some(idx.len);
        self.fetched_idx = Some(idx);
        Ok(())
    }

    /// Read the fetched sequence into the given vector.
    pub fn read(&mut self, seq: &mut Text) -> io::Result<()> {
        let idx = self.fetched_idx.clone();
        match (idx, self.start, self.stop) {
            (Some(idx), Some(start), Some(stop)) => self.read_into_buffer(idx, start, stop, seq),
            _ => Err(io::Error::new(
                io::ErrorKind::Other,
                "No sequence fetched for reading.",
            )),
        }
    }

    /// Return an iterator yielding the fetched sequence.
    pub fn read_iter(&mut self) -> io::Result<IndexedReaderIterator<'_, R>> {
        let idx = self.fetched_idx.clone();
        match (idx, self.start, self.stop) {
            (Some(idx), Some(start), Some(stop)) => self.read_into_iter(idx, start, stop),
            _ => Err(io::Error::new(
                io::ErrorKind::Other,
                "No sequence fetched for reading.",
            )),
        }
    }

    fn read_into_buffer(
        &mut self,
        idx: IndexRecord,
        start: u64,
        stop: u64,
        seq: &mut Text,
    ) -> io::Result<()> {
        if stop > idx.len {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "FASTA read interval was out of bounds",
            ));
        } else if start > stop {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Invalid query interval",
            ));
        }

        let mut bases_left = stop - start;
        let mut line_offset = self.seek_to(&idx, start)?;

        seq.clear();
        while bases_left > 0 {
            bases_left -= self.read_line(&idx, &mut line_offset, bases_left, seq)?;
        }

        Ok(())
    }

    fn read_into_iter(
        &mut self,
        idx: IndexRecord,
        start: u64,
        stop: u64,
    ) -> io::Result<IndexedReaderIterator<'_, R>> {
        if stop > idx.len {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "FASTA read interval was out of bounds",
            ));
        } else if start > stop {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Invalid query interval",
            ));
        }

        let bases_left = stop - start;
        let line_offset = self.seek_to(&idx, start)?;
        let capacity = min(
            MAX_FASTA_BUFFER_SIZE,
            min(bases_left, idx.line_bases) as usize,
        );

        Ok(IndexedReaderIterator {
            reader: self,
            record: idx,
            bases_left,
            line_offset,
            buf: Vec::with_capacity(capacity),
            buf_idx: 0,
        })
    }

    /// Return the IndexRecord for the given sequence name or io::Result::Err
    fn idx(&self, seqname: &str) -> io::Result<IndexRecord> {
        match self.index.name_to_rid.get(seqname) {
            Some(rid) => self.idx_by_rid(*rid),
            None => Err(io::Error::new(
                io::ErrorKind::Other,
                "Unknown sequence name.",
            )),
        }
    }

    /// Return the IndexRecord for the given record index or io::Result::Err
    fn idx_by_rid(&self, rid: usize) -> io::Result<IndexRecord> {
        match self.index.inner.get(rid) {
            Some(record) => Ok(record.clone()),
            None => Err(io::Error::new(
                io::ErrorKind::Other,
                "Invalid record index in fasta file.",
            )),
        }
    }

    /// Seek to the given position in the specified FASTA record. The position
    /// of the cursor on the line that the seek ended on is returned.
    fn seek_to(&mut self, idx: &IndexRecord, start: u64) -> io::Result<u64> {
        assert!(start <= idx.len);

        let line_offset = start % idx.line_bases;
        let line_start = start / idx.line_bases * idx.line_bytes;
        let offset = idx.offset + line_start + line_offset;
        r#try!(self.reader.seek(io::SeekFrom::Start(offset)));

        Ok(line_offset)
    }

    /// Tries to read up to `bases_left` bases from the current line into `buf`,
    /// returning the actual number of bases read. Depending on the amount of
    /// whitespace per line, the current `line_offset`, and the amount of bytes
    /// returned from `BufReader::fill_buf`, this function may return Ok(0)
    /// multiple times in a row.
    fn read_line(
        &mut self,
        idx: &IndexRecord,
        line_offset: &mut u64,
        bases_left: u64,
        buf: &mut Vec<u8>,
    ) -> io::Result<u64> {
        let (bytes_to_read, bytes_to_keep) = {
            let src = self.reader.fill_buf()?;
            if src.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "FASTA file is truncated.",
                ));
            }

            let bases_on_line = idx.line_bases - min(idx.line_bases, *line_offset);
            let bases_in_buffer = min(src.len() as u64, bases_on_line);

            let (bytes_to_read, bytes_to_keep) = if bases_in_buffer <= bases_left {
                let bytes_to_read = min(src.len() as u64, idx.line_bytes - *line_offset);

                (bytes_to_read, bases_in_buffer)
            } else {
                (bases_left, bases_left)
            };

            buf.extend_from_slice(&src[..bytes_to_keep as usize]);
            (bytes_to_read, bytes_to_keep)
        };

        self.reader.consume(bytes_to_read as usize);

        assert!(bytes_to_read > 0);
        *line_offset += bytes_to_read;
        if *line_offset >= idx.line_bytes {
            *line_offset = 0;
        }

        Ok(bytes_to_keep)
    }
}

/// Record of a FASTA index.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexRecord {
    name: String,
    len: u64,
    offset: u64,
    line_bases: u64,
    line_bytes: u64,
}

/// A sequence record returned by the FASTA index.
#[derive(Debug, PartialEq)]
pub struct Sequence {
    pub name: String,
    pub len: u64,
}

pub struct IndexedReaderIterator<'a, R: io::Read + io::Seek> {
    reader: &'a mut IndexedReader<R>,
    record: IndexRecord,
    bases_left: u64,
    line_offset: u64,
    buf: Vec<u8>,
    buf_idx: usize,
}

impl<'a, R: io::Read + io::Seek + 'a> IndexedReaderIterator<'a, R> {
    fn fill_buffer(&mut self) -> io::Result<()> {
        assert!(self.bases_left > 0);

        self.buf.clear();
        let bases_to_read = min(self.buf.capacity() as u64, self.bases_left);

        // May loop one or more times; see IndexedReader::read_line.
        while self.buf.is_empty() {
            self.bases_left -= self.reader.read_line(
                &self.record,
                &mut self.line_offset,
                bases_to_read,
                &mut self.buf,
            )?;
        }

        self.buf_idx = 0;
        Ok(())
    }
}

impl<'a, R: io::Read + io::Seek + 'a> Iterator for IndexedReaderIterator<'a, R> {
    type Item = io::Result<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf_idx < self.buf.len() {
            let item = Some(Ok(self.buf[self.buf_idx]));
            self.buf_idx += 1;
            item
        } else if self.bases_left > 0 {
            if let Err(e) = self.fill_buffer() {
                self.bases_left = 0;
                self.buf_idx = self.buf.len();

                return Some(Err(e));
            }

            self.buf_idx = 1;
            Some(Ok(self.buf[0]))
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let hint = self.bases_left as usize + (self.buf.len() - self.buf_idx);

        (hint, Some(hint))
    }
}

/// A FASTA writer.
#[derive(Debug)]
pub struct Writer<W: io::Write> {
    writer: io::BufWriter<W>,
}

impl Writer<fs::File> {
    /// Write to the given file path.
    pub fn to_file<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        fs::File::create(path).map(Writer::new)
    }
}

impl<W: io::Write> Writer<W> {
    /// Create a new FASTA writer.
    pub fn new(writer: W) -> Self {
        Writer {
            writer: io::BufWriter::new(writer),
        }
    }

    /// Directly write a FASTA record.
    pub fn write_record(&mut self, record: &Record) -> io::Result<()> {
        self.write(record.id(), record.desc(), record.seq())
    }

    /// Write a FASTA record with given id, optional description and sequence.
    pub fn write(&mut self, id: &str, desc: Option<&str>, seq: TextSlice<'_>) -> io::Result<()> {
        r#try!(self.writer.write_all(b">"));
        r#try!(self.writer.write_all(id.as_bytes()));
        if desc.is_some() {
            r#try!(self.writer.write_all(b" "));
            r#try!(self.writer.write_all(desc.unwrap().as_bytes()));
        }
        r#try!(self.writer.write_all(b"\n"));
        r#try!(self.writer.write_all(seq));
        r#try!(self.writer.write_all(b"\n"));

        Ok(())
    }

    /// Flush the writer, ensuring that everything is written.
    pub fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// A FASTA record.
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    id: String,
    desc: Option<String>,
    seq: String,
}

impl Record {
    /// Create a new instance.
    pub fn new() -> Self {
        Record {
            id: String::new(),
            desc: None,
            seq: String::new(),
        }
    }

    /// Create a new FASTA record from given attributes.
    pub fn with_attrs(id: &str, desc: Option<&str>, seq: TextSlice<'_>) -> Self {
        let desc = match desc {
            Some(desc) => Some(desc.to_owned()),
            _ => None,
        };
        Record {
            id: id.to_owned(),
            desc,
            seq: String::from_utf8(seq.to_vec()).unwrap(),
        }
    }

    /// Check if record is empty.
    pub fn is_empty(&self) -> bool {
        self.id.is_empty() && self.desc.is_none() && self.seq.is_empty()
    }

    /// Check validity of FASTA record.
    pub fn check(&self) -> Result<(), &str> {
        if self.id().is_empty() {
            return Err("Expecting id for Fasta record.");
        }
        if !self.seq.is_ascii() {
            return Err("Non-ascii character found in sequence.");
        }

        Ok(())
    }

    /// Return the id of the record.
    pub fn id(&self) -> &str {
        self.id.as_ref()
    }

    /// Return descriptions if present.
    pub fn desc(&self) -> Option<&str> {
        match self.desc.as_ref() {
            Some(desc) => Some(&desc),
            None => None,
        }
    }

    /// Return the sequence of the record.
    pub fn seq(&self) -> TextSlice<'_> {
        self.seq.as_bytes()
    }

    /// Clear the record.
    fn clear(&mut self) {
        self.id.clear();
        self.desc = None;
        self.seq.clear();
    }
}

/// An iterator over the records of a FASTA file.
pub struct Records<R: io::Read> {
    reader: Reader<R>,
    error_has_occured: bool,
}

impl<R: io::Read> Iterator for Records<R> {
    type Item = io::Result<Record>;

    fn next(&mut self) -> Option<io::Result<Record>> {
        if self.error_has_occured {
            None
        } else {
            let mut record = Record::new();
            match self.reader.read(&mut record) {
                Ok(()) if record.is_empty() => None,
                Ok(()) => Some(Ok(record)),
                Err(err) => {
                    self.error_has_occured = true;
                    Some(Err(err))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    const FASTA_FILE: &'static [u8] = b">id desc
ACCGTAGGCTGA
CCGTAGGCTGAA
CGTAGGCTGAAA
GTAGGCTGAAAA
CCCC
>id2
ATTGTTGTTTTA
ATTGTTGTTTTA
ATTGTTGTTTTA
GGGG
";
    const FAI_FILE: &'static [u8] = b"id\t52\t9\t12\t13
id2\t40\t71\t12\t13
";

    const TRUNCATED_FASTA: &'static [u8] = b">id desc\nACCGTAGGCTGA";

    const FASTA_FILE_CRLF: &'static [u8] = b">id desc\r
ACCGTAGGCTGA\r
CCGTAGGCTGAA\r
CGTAGGCTGAAA\r
GTAGGCTGAAAA\r
CCCC\r
>id2\r
ATTGTTGTTTTA\r
ATTGTTGTTTTA\r
ATTGTTGTTTTA\r
GGGG\r
";
    const FAI_FILE_CRLF: &'static [u8] = b"id\t52\t10\t12\t14\r
id2\t40\t78\t12\t14\r
";

    const FASTA_FILE_NO_TRAILING_LF: &'static [u8] = b">id desc
GTAGGCTGAAAA
CCCC";
    const FAI_FILE_NO_TRAILING_LF: &'static [u8] = b"id\t16\t9\t12\t13";

    const WRITE_FASTA_FILE: &'static [u8] = b">id desc
ACCGTAGGCTGA
>id2
ATTGTTGTTTTA
";

    struct ReaderMock {
        seek_fails: bool,
        read_fails: bool,
    }

    impl Read for ReaderMock {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.read_fails {
                Err(io::Error::new(io::ErrorKind::Other, "Read set to fail"))
            } else {
                Ok(buf.len())
            }
        }
    }

    impl Seek for ReaderMock {
        fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
            if let io::SeekFrom::Start(pos) = pos {
                if self.seek_fails {
                    Err(io::Error::new(io::ErrorKind::Other, "Seek set to fail"))
                } else {
                    Ok(pos)
                }
            } else {
                unimplemented!();
            }
        }
    }

    #[test]
    fn test_reader() {
        let reader = Reader::new(FASTA_FILE);
        let ids = ["id", "id2"];
        let descs = [Some("desc"), None];
        let seqs: [&[u8]; 2] = [
            b"ACCGTAGGCTGACCGTAGGCTGAACGTAGGCTGAAAGTAGGCTGAAAACCCC",
            b"ATTGTTGTTTTAATTGTTGTTTTAATTGTTGTTTTAGGGG",
        ];

        for (i, r) in reader.records().enumerate() {
            let record = r.expect("Error reading record");
            assert_eq!(record.check(), Ok(()));
            assert_eq!(record.id(), ids[i]);
            assert_eq!(record.desc(), descs[i]);
            assert_eq!(record.seq(), seqs[i]);
        }
    }

    #[test]
    fn test_faread_trait() {
        let path = "genome.fa.gz";
        let mut fa_reader: Box<dyn FastaRead> = match path.ends_with(".gz") {
            true => Box::new(Reader::new(io::BufReader::new(FASTA_FILE))),
            false => Box::new(Reader::new(FASTA_FILE)),
        };
        // The read method can be called, since it is implemented by
        // FQRead. Right now, the records method would not work.
        let mut record = Record::new();
        fa_reader.read(&mut record).unwrap();
        // Check if the returned result is correct.
        assert_eq!(record.check(), Ok(()));
        assert_eq!(record.id(), "id");
        assert_eq!(record.desc(), Some("desc"));
        assert_eq!(
            record.seq().to_vec(),
            b"ACCGTAGGCTGACCGTAGGCTGAACGTAGGCTGAAAGTAGGCTGAAAACCCC".to_vec()
        );
    }

    #[test]
    fn test_reader_wrong_header() {
        let mut reader = Reader::new(&b"!test\nACGTA\n"[..]);
        let mut record = Record::new();
        assert!(
            reader.read(&mut record).is_err(),
            "read() should return Err if FASTA header is malformed"
        );
    }

    #[test]
    fn test_reader_no_id() {
        let mut reader = Reader::new(&b">\nACGTA\n"[..]);
        let mut record = Record::new();
        reader.read(&mut record).unwrap();
        assert!(
            record.check().is_err(),
            "check() should return Err if FASTA header is empty"
        );
    }

    #[test]
    fn test_reader_non_ascii_sequence() {
        let mut reader = Reader::new(&b">id\nACGTA\xE2\x98\xB9AT\n"[..]);
        let mut record = Record::new();
        reader.read(&mut record).unwrap();
        assert!(
            record.check().is_err(),
            "check() should return Err if FASTA sequence is not ASCII"
        );
    }

    #[test]
    fn test_reader_read_fails() {
        let mut reader = Reader::new(ReaderMock {
            seek_fails: false,
            read_fails: true,
        });
        let mut record = Record::new();
        assert!(
            reader.read(&mut record).is_err(),
            "read() should return Err if Read::read fails"
        );
    }

    #[test]
    fn test_reader_read_fails_iter() {
        let reader = Reader::new(ReaderMock {
            seek_fails: false,
            read_fails: true,
        });
        let mut records = reader.records();

        assert!(
            records.next().unwrap().is_err(),
            "next() should return Err if Read::read fails"
        );
        assert!(
            records.next().is_none(),
            "next() should return None after error has occurred"
        );
    }

    #[test]
    fn test_record_with_attrs() {
        let record = Record::with_attrs("id_str", Some("desc"), b"ATGCGGG");
        assert_eq!(record.id(), "id_str");
        assert_eq!(record.desc(), Some("desc"));
        assert_eq!(record.seq(), b"ATGCGGG");
    }

    #[test]
    fn test_index_sequences() {
        let reader = IndexedReader::new(io::Cursor::new(FASTA_FILE), FAI_FILE).unwrap();

        let sequences = reader.index.sequences();
        assert_eq!(sequences.len(), 2);
        assert_eq!(
            sequences[0],
            Sequence {
                name: "id".into(),
                len: 52,
            }
        );
        assert_eq!(
            sequences[1],
            Sequence {
                name: "id2".into(),
                len: 40,
            }
        );
    }

    #[test]
    fn test_indexed_reader() {
        _test_indexed_reader(&FASTA_FILE, &FAI_FILE, _read_buffer);
        _test_indexed_reader_truncated(_read_buffer);
        _test_indexed_reader_extreme_whitespace(_read_buffer);
    }

    #[test]
    fn test_indexed_reader_crlf() {
        _test_indexed_reader(&FASTA_FILE_CRLF, &FAI_FILE_CRLF, _read_buffer);
    }

    #[test]
    fn test_indexed_reader_iter() {
        _test_indexed_reader(&FASTA_FILE, &FAI_FILE, _read_iter);
        _test_indexed_reader_truncated(_read_iter);
        _test_indexed_reader_extreme_whitespace(_read_iter);
    }

    #[test]
    fn test_indexed_reader_iter_crlf() {
        _test_indexed_reader(&FASTA_FILE_CRLF, &FAI_FILE_CRLF, _read_iter);
    }

    fn _test_indexed_reader<'a, F>(fasta: &'a [u8], fai: &'a [u8], read: F)
    where
        F: Fn(&mut IndexedReader<io::Cursor<&'a [u8]>>, &str, u64, u64) -> io::Result<Vec<u8>>,
    {
        let mut reader = IndexedReader::new(io::Cursor::new(fasta), fai).unwrap();

        // Test reading various substrings of the sequence
        assert_eq!(read(&mut reader, "id", 1, 5).unwrap(), b"CCGT");
        assert_eq!(
            read(&mut reader, "id", 1, 31).unwrap(),
            b"CCGTAGGCTGACCGTAGGCTGAACGTAGGC"
        );
        assert_eq!(read(&mut reader, "id", 13, 23).unwrap(), b"CGTAGGCTGA");
        assert_eq!(
            read(&mut reader, "id", 36, 52).unwrap(),
            b"GTAGGCTGAAAACCCC"
        );
        assert_eq!(
            read(&mut reader, "id2", 12, 40).unwrap(),
            b"ATTGTTGTTTTAATTGTTGTTTTAGGGG"
        );
        assert_eq!(read(&mut reader, "id2", 12, 12).unwrap(), b"");
        assert_eq!(read(&mut reader, "id2", 12, 13).unwrap(), b"A");
        // Minimal sequence spanning new-line
        assert_eq!(read(&mut reader, "id", 11, 13).unwrap(), b"AC");

        assert!(read(&mut reader, "id2", 12, 11).is_err());
        assert!(read(&mut reader, "id2", 12, 1000).is_err());
        assert!(read(&mut reader, "id3", 0, 1).is_err());
    }

    fn _test_indexed_reader_truncated<'a, F>(read: F)
    where
        F: Fn(&mut IndexedReader<io::Cursor<&'a [u8]>>, &str, u64, u64) -> io::Result<Vec<u8>>,
    {
        let mut reader = IndexedReader::new(io::Cursor::new(TRUNCATED_FASTA), FAI_FILE).unwrap();

        assert_eq!(read(&mut reader, "id", 0, 12).unwrap(), b"ACCGTAGGCTGA");
        assert!(read(&mut reader, "id", 0, 13).is_err()); // read past EOF
        assert!(read(&mut reader, "id", 36, 52).is_err()); // seek and read past EOF
        assert!(read(&mut reader, "id2", 12, 40).is_err()); // seek and read past EOF
    }

    fn _test_indexed_reader_extreme_whitespace<'a, F>(read: F)
    where
        F: Fn(&mut IndexedReader<io::Cursor<Vec<u8>>>, &str, u64, u64) -> io::Result<Vec<u8>>,
    {
        // Test to exercise the case where we cannot consume all whitespace at once. More than
        // DEFAULT_BUF_SIZE (a non-public constant set to 8 * 1024) whitespace is used to ensure
        // that it can't all fit in the BufReader at once.
        let mut seq = Vec::new();
        seq.push(b'A');
        seq.resize(10000, b' ');
        seq.push(b'B');

        let fasta = io::Cursor::new(seq);
        let fai = io::Cursor::new(Vec::from(&b"id\t2\t0\t1\t10000"[..]));
        let mut reader = IndexedReader::new(fasta, fai).unwrap();

        assert_eq!(read(&mut reader, "id", 0, 2).unwrap(), b"AB");
    }

    fn _read_buffer<T>(
        reader: &mut IndexedReader<T>,
        seqname: &str,
        start: u64,
        stop: u64,
    ) -> io::Result<Vec<u8>>
    where
        T: Seek + Read,
    {
        let mut seq = vec![];
        reader.fetch(seqname, start, stop)?;
        reader.read(&mut seq)?;

        Ok(seq)
    }

    fn _read_iter<T>(
        reader: &mut IndexedReader<T>,
        seqname: &str,
        start: u64,
        stop: u64,
    ) -> io::Result<Vec<u8>>
    where
        T: Seek + Read,
    {
        let mut seq = vec![];
        reader.fetch(seqname, start, stop)?;
        for nuc in reader.read_iter()? {
            seq.push(nuc?);
        }

        Ok(seq)
    }

    #[test]
    fn test_indexed_reader_all() {
        _test_indexed_reader_all(&FASTA_FILE, &FAI_FILE, _read_buffer_all);
    }

    #[test]
    fn test_indexed_reader_crlf_all() {
        _test_indexed_reader_all(&FASTA_FILE_CRLF, &FAI_FILE_CRLF, _read_buffer_all);
    }

    #[test]
    fn test_indexed_reader_iter_all() {
        _test_indexed_reader_all(&FASTA_FILE, &FAI_FILE, _read_iter_all);
    }

    #[test]
    fn test_indexed_reader_iter_crlf_all() {
        _test_indexed_reader_all(&FASTA_FILE_CRLF, &FAI_FILE_CRLF, _read_iter_all);
    }

    fn _test_indexed_reader_all<'a, F>(fasta: &'a [u8], fai: &'a [u8], read: F)
    where
        F: Fn(&mut IndexedReader<io::Cursor<&'a [u8]>>, &str) -> io::Result<Vec<u8>>,
    {
        let mut reader = IndexedReader::new(io::Cursor::new(fasta), fai).unwrap();

        assert_eq!(
            read(&mut reader, "id").unwrap(),
            &b"ACCGTAGGCTGACCGTAGGCTGAACGTAGGCTGAAAGTAGGCTGAAAACCCC"[..]
        );
        assert_eq!(
            read(&mut reader, "id2").unwrap(),
            &b"ATTGTTGTTTTAATTGTTGTTTTAATTGTTGTTTTAGGGG"[..]
        );
    }

    fn _read_buffer_all<T>(reader: &mut IndexedReader<T>, seqname: &str) -> io::Result<Vec<u8>>
    where
        T: Seek + Read,
    {
        let mut seq = vec![];
        reader.fetch_all(seqname)?;
        reader.read(&mut seq)?;

        Ok(seq)
    }

    fn _read_iter_all<T>(reader: &mut IndexedReader<T>, seqname: &str) -> io::Result<Vec<u8>>
    where
        T: Seek + Read,
    {
        let mut seq = vec![];
        reader.fetch_all(seqname)?;
        for nuc in reader.read_iter()? {
            seq.push(nuc?);
        }

        Ok(seq)
    }

    #[test]
    fn test_indexed_reader_by_rid_all() {
        _test_indexed_reader_by_rid_all(&FASTA_FILE, &FAI_FILE, _read_buffer_by_rid_all);
    }

    #[test]
    fn test_indexed_reader_crlf_by_rid_all() {
        _test_indexed_reader_by_rid_all(&FASTA_FILE_CRLF, &FAI_FILE_CRLF, _read_buffer_by_rid_all);
    }

    #[test]
    fn test_indexed_reader_iter_by_rid_all() {
        _test_indexed_reader_by_rid_all(&FASTA_FILE, &FAI_FILE, _read_iter_by_rid_all);
    }

    #[test]
    fn test_indexed_reader_iter_crlf_by_rid_all() {
        _test_indexed_reader_by_rid_all(&FASTA_FILE_CRLF, &FAI_FILE_CRLF, _read_iter_by_rid_all);
    }

    fn _test_indexed_reader_by_rid_all<'a, F>(fasta: &'a [u8], fai: &'a [u8], read: F)
    where
        F: Fn(&mut IndexedReader<io::Cursor<&'a [u8]>>, usize) -> io::Result<Vec<u8>>,
    {
        let mut reader = IndexedReader::new(io::Cursor::new(fasta), fai).unwrap();

        assert_eq!(
            read(&mut reader, 0).unwrap(),
            &b"ACCGTAGGCTGACCGTAGGCTGAACGTAGGCTGAAAGTAGGCTGAAAACCCC"[..]
        );
        assert_eq!(
            read(&mut reader, 1).unwrap(),
            &b"ATTGTTGTTTTAATTGTTGTTTTAATTGTTGTTTTAGGGG"[..]
        );
    }

    fn _read_buffer_by_rid_all<T>(
        reader: &mut IndexedReader<T>,
        seq_index: usize,
    ) -> io::Result<Vec<u8>>
    where
        T: Seek + Read,
    {
        let mut seq = vec![];
        reader.fetch_all_by_rid(seq_index)?;
        reader.read(&mut seq)?;

        Ok(seq)
    }

    fn _read_iter_by_rid_all<T>(
        reader: &mut IndexedReader<T>,
        seq_index: usize,
    ) -> io::Result<Vec<u8>>
    where
        T: Seek + Read,
    {
        let mut seq = vec![];
        reader.fetch_all_by_rid(seq_index)?;
        for nuc in reader.read_iter()? {
            seq.push(nuc?);
        }

        Ok(seq)
    }

    #[test]
    fn test_indexed_reader_iter_size_hint() {
        let mut reader = IndexedReader::new(io::Cursor::new(FASTA_FILE), FAI_FILE).unwrap();
        reader.fetch("id", 2, 4).unwrap();
        let mut iterator = reader.read_iter().unwrap();

        assert_eq!(iterator.size_hint(), (2, Some(2)));
        assert_eq!(iterator.next().unwrap().unwrap(), b'C');
        assert_eq!(iterator.size_hint(), (1, Some(1)));
        assert_eq!(iterator.next().unwrap().unwrap(), b'G');
        assert_eq!(iterator.size_hint(), (0, Some(0)));
        assert!(iterator.next().is_none());
        assert_eq!(iterator.size_hint(), (0, Some(0)));
    }

    #[test]
    fn test_indexed_reader_reused_buffer() {
        let mut reader = IndexedReader::new(io::Cursor::new(FASTA_FILE), FAI_FILE).unwrap();
        let mut seq = Vec::new();

        reader.fetch("id", 1, 5).unwrap();
        reader.read(&mut seq).unwrap();
        assert_eq!(seq, b"CCGT");

        reader.fetch("id", 13, 23).unwrap();
        reader.read(&mut seq).unwrap();
        assert_eq!(seq, b"CGTAGGCTGA");
    }

    #[test]
    fn test_indexed_reader_no_trailing_lf() {
        let mut reader = IndexedReader::new(
            io::Cursor::new(FASTA_FILE_NO_TRAILING_LF),
            FAI_FILE_NO_TRAILING_LF,
        )
        .unwrap();
        let mut seq = Vec::new();

        reader.fetch("id", 0, 16).unwrap();
        reader.read(&mut seq).unwrap();
        assert_eq!(seq, b"GTAGGCTGAAAACCCC");
    }

    #[test]
    fn test_indexed_reader_bad_reader() {
        let bad_reader = ReaderMock {
            seek_fails: false,
            read_fails: false,
        };
        let mut reader = IndexedReader::new(bad_reader, FAI_FILE).unwrap();
        let mut seq = Vec::new();
        reader.fetch("id", 0, 10).unwrap();
        assert!(reader.read(&mut seq).is_ok())
    }

    #[test]
    fn test_indexed_reader_read_seek_fails() {
        let bad_reader = ReaderMock {
            seek_fails: true,
            read_fails: false,
        };
        let mut reader = IndexedReader::new(bad_reader, FAI_FILE).unwrap();
        let mut seq = Vec::new();
        reader.fetch("id", 0, 10).unwrap();
        assert!(reader.read(&mut seq).is_err());
    }

    #[test]
    fn test_indexed_reader_read_read_fails() {
        let bad_reader = ReaderMock {
            seek_fails: false,
            read_fails: true,
        };
        let mut reader = IndexedReader::new(bad_reader, FAI_FILE).unwrap();
        let mut seq = Vec::new();
        reader.fetch("id", 0, 10).unwrap();
        assert!(reader.read(&mut seq).is_err());
    }

    #[test]
    fn test_indexed_reader_iter_seek_fails() {
        let bad_reader = ReaderMock {
            seek_fails: true,
            read_fails: false,
        };
        let mut reader = IndexedReader::new(bad_reader, FAI_FILE).unwrap();
        reader.fetch("id", 0, 10).unwrap();
        assert!(reader.read_iter().is_err());
    }

    #[test]
    fn test_indexed_reader_iter_read_fails() {
        let bad_reader = ReaderMock {
            seek_fails: false,
            read_fails: true,
        };
        let mut reader = IndexedReader::new(bad_reader, FAI_FILE).unwrap();
        reader.fetch("id", 0, 10).unwrap();
        let mut iterator = reader.read_iter().unwrap();
        assert!(iterator.next().unwrap().is_err());
        assert!(
            iterator.next().is_none(),
            "next() should return none after error has occurred"
        );
    }

    #[test]
    fn test_indexed_reader_no_fetch_read_fails() {
        let reader = ReaderMock {
            seek_fails: false,
            read_fails: false,
        };
        let mut reader = IndexedReader::new(reader, FAI_FILE).unwrap();
        let mut seq = vec![];
        assert!(reader.read(&mut seq).is_err());
    }

    #[test]
    fn test_indexed_reader_no_fetch_read_iter_fails() {
        let reader = ReaderMock {
            seek_fails: false,
            read_fails: false,
        };
        let mut reader = IndexedReader::new(reader, FAI_FILE).unwrap();
        assert!(reader.read_iter().is_err());
    }

    #[test]
    fn test_writer() {
        let mut writer = Writer::new(Vec::new());
        writer.write("id", Some("desc"), b"ACCGTAGGCTGA").unwrap();
        writer.write("id2", None, b"ATTGTTGTTTTA").unwrap();
        writer.flush().unwrap();
        assert_eq!(writer.writer.get_ref(), &WRITE_FASTA_FILE);
    }
}
