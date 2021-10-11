use anyhow;
use anyhow::{Context, Result};
use bytes::Bytes;
use std::fs::File;
use std::io::Read;
use std::os::unix::prelude::OsStrExt;
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};

#[derive(PartialOrd, Ord, PartialEq, Eq, Debug)]
pub enum Input<'a> {
    /// Input file.
    File(&'a OsString),
    /// string uniquely defining the tool version (could be even the hash of its binary).    
    ToolTag(&'a OsString),
}

/// Input set is the set of all inputs to the build step.
#[derive(Default, Debug)]
pub struct InputSet<'a> {
    pub inputs: Vec<Input<'a>>,
}

#[derive(Debug, Default)]
pub struct HashBundle<'a> {
    pub hash: String,
    pub input_hashes: Vec<(&'a Input<'a>, String)>,
}

// TODO: should we also add exec bit?
#[derive(PartialOrd, Ord, PartialEq, Eq)]
pub struct FileOutput<'a> {
    filename: &'a OsString,
    present: bool,
    contents: Bytes,
}

#[derive(PartialOrd, Ord, PartialEq, Eq)]
pub enum Output<'a> {
    File(&'a FileOutput<'a>),
    Stdout(&'a OsString),
    Stderr(&'a OsString),
    Log(&'a FileOutput<'a>),
}

/// Output set is the set of all process outputs.
#[derive(Default)]
pub struct OutputSet<'a> {
    pub outputs: Vec<(Output<'a>, bool)>, // The bool indicates whether we store this output in the cache.
}

/// Returns the hash of the given file.
///
/// TODO(valeryz): Cache these in a parent process' memory by the
/// output of stat(2), except atime, so that we don't have to read
/// them twice during a single build process.
fn file_hash(filename: &OsStr) -> Result<String> {
    const BUFSIZE: usize = 4096;
    let mut acc = Sha256::new();
    let mut f =
        File::open(filename).with_context(|| format!("Reading input file {:?}", filename))?;
    let mut buf: [u8; BUFSIZE] = [0; BUFSIZE];
    loop {
        let rd = f.read(&mut buf)?;
        if rd == 0 {
            break;
        }
        acc.update(&buf[..rd]);
    }
    Ok(format!("{:x}", acc.finalize()))
}

fn string_hash(s: &OsStr) -> String {
    let mut acc = Sha256::new();
    acc.update(s.as_bytes());
    format!("{:x}", acc.finalize())
}

impl<'a> InputSet<'a> {
    /// Returns the HEX string of the hash of the whole input set.
    ///
    /// We calculate the whole hash bundle, and discard the separate hashes.
    pub fn hash(&'a self) -> Result<String> {
        self.hash_bundle().map(|x| x.hash)
    }

    /// Returns the HEX string of the hash of the files in the input set, and the total hash.
    ///
    /// It does this by calculating a SHA256 hash of all SHA256 hashes of inputs (being either file
    /// or tool tag) sorted by the values of the hashes themselves.
    pub fn hash_bundle(&'a self) -> Result<HashBundle<'a>> {
        // Calculate the hash of the input set independently of the order.
        let mut hash_bundle = HashBundle::default();
        for input in &self.inputs {
            match input {
                Input::File(s) => {
                    hash_bundle.input_hashes.push((input, format!("File{}", file_hash(s)?)));
                }
                Input::ToolTag(s) => {
                    hash_bundle.input_hashes.push((input, format!("ToolTag{}", string_hash(s))));
                }
            }
        }
        // Sort inputs hashes by the hash value.
        hash_bundle.input_hashes.sort_by(|a, b| a.1.cmp(&b.1));
        let mut acc: Sha256 = Sha256::new();
        for hash in hash_bundle.input_hashes.iter() {
            acc.update(&hash.1);
        }
        hash_bundle.hash = format!("{:x}", acc.finalize());
        Ok(hash_bundle)
    }

    pub fn add_input(&mut self, input: Input<'a>) {
        self.inputs.push(input)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    const EMPTY_SHA256: &'static str =
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn file_hash_test() -> Result<()> {
        let file = NamedTempFile::new()?;
        let hash = file_hash(file.path().as_os_str())?;
        // Sha256 hash of an empty file.
        assert_eq!(hash, EMPTY_SHA256);
        Ok(())
    }

    #[test]
    fn file_hash_nonexistent() {
        assert!(file_hash(&OsString::from("/nonexistent-capsule-input")).is_err());
    }

    #[test]
    fn test_input_set_empty() {
        let input_set = InputSet::default();
        assert_eq!(input_set.hash().unwrap(), EMPTY_SHA256);
    }

    #[test]
    fn test_input_set_1() {
        let mut input_set = InputSet::default();
        let tool_tag = OsString::from("some tool_tag");
        input_set.add_input(Input::ToolTag(&tool_tag));
        let hash1 = input_set.hash().unwrap();
        assert_ne!(hash1, EMPTY_SHA256);
        let hash2 = input_set.hash().unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_input_set_different_order() {
        let mut input_set1 = InputSet::default();
        let tool_tag1 = OsString::from("some tool_tag");
        let tool_tag2 = OsString::from("another tool_tag");
        input_set1.add_input(Input::ToolTag(&tool_tag1));
        input_set1.add_input(Input::ToolTag(&tool_tag2));
        let mut input_set2 = InputSet::default();
        input_set2.add_input(Input::ToolTag(&tool_tag2));
        input_set2.add_input(Input::ToolTag(&tool_tag1));
        assert_eq!(input_set1.hash().unwrap(), input_set2.hash().unwrap());
    }

    #[test]
    fn test_input_set_bundle() {
        let mut input_set1 = InputSet::default();
        let tool_tag1 = OsString::from("some tool_tag");
        let tool_tag2 = OsString::from("another tool_tag");
        input_set1.add_input(Input::ToolTag(&tool_tag1));
        input_set1.add_input(Input::ToolTag(&tool_tag2));
        let mut input_set2 = InputSet::default();
        input_set2.add_input(Input::ToolTag(&tool_tag2));
        input_set2.add_input(Input::ToolTag(&tool_tag1));
        let bundle1 = input_set1.hash_bundle().unwrap();
        let bundle2 = input_set2.hash_bundle().unwrap();
        assert_eq!(bundle1.hash, bundle2.hash);
        assert_eq!(bundle1.input_hashes, bundle2.input_hashes);
    }


    #[test]
    fn test_input_set_file() {
        let mut file1 = NamedTempFile::new().unwrap();
        file1.write("file1".as_bytes()).unwrap();
        file1.flush().unwrap();
        let mut file2 = NamedTempFile::new().unwrap();
        file2.write("file2".as_bytes()).unwrap();
        file2.flush().unwrap();
        let mut input_set = InputSet::default();
        let path1 = OsString::from(file1.path());
        input_set.add_input(Input::File(&path1));
        // These hashes were obtained by manual manipulation files and `openssl sha256`
        assert_eq!(
            input_set.hash().unwrap(),
            "f409e4c7ae76997e69556daae6139bee1f02e4f618d3da8deea10bb35b6c0ebd"
        );
        let path2 = OsString::from(file2.path());
        input_set.add_input(Input::File(&path2));
        assert_eq!(
            input_set.hash().unwrap(),
            "a282f3da61a4bc322a8d31da6d30a0e924017962acbef2f6996b81709de8cdc3"
        );
    }
}
