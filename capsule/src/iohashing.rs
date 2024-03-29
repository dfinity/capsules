use anyhow;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::workspace_path::WorkspacePath;

#[derive(PartialOrd, Ord, PartialEq, Eq, Debug, Clone, Serialize, Deserialize)]
pub enum Input {
    /// string uniquely defining the tool version (could be even the hash of its binary).    
    ToolTag(String),
    /// Input file.
    File(WorkspacePath),
}

/// Input set is the set of all inputs to the build step.
#[derive(Default, Debug, Clone)]
pub struct InputSet {
    pub inputs: Vec<Input>,
}

#[derive(PartialOrd, Ord, PartialEq, Eq, Debug, Clone, Serialize, Deserialize)]
pub struct FileOutput {
    pub filename: WorkspacePath,
    pub present: bool,
    pub mode: u32,
}

#[derive(PartialOrd, Ord, PartialEq, Eq, Debug, Clone, Serialize, Deserialize)]
pub enum Output {
    File(FileOutput),
    ExitCode(i32),
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct InputHashBundle {
    pub hash: String,
    pub hash_details: Vec<(Input, String)>,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct OutputHashBundle {
    pub hash: String,
    pub hash_details: Vec<(Output, String)>,
}

impl OutputHashBundle {
    // Find the result code in all the fields.
    pub fn result_code(&self) -> Option<i32> {
        for (output, _) in &self.hash_details {
            if let Output::ExitCode(code) = output {
                return Some(*code);
            }
        }
        None
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct InputOutputBundle {
    pub inputs: InputHashBundle,
    pub outputs: OutputHashBundle,
    pub source: String,
}

/// Output set is the set of all process outputs.
#[derive(Default)]
pub struct OutputSet {
    pub outputs: Vec<Output>,
}

/// Returns the hash of the given file.
///
/// TODO(valeryz): Maybe cache these in a parent process' memory by the
/// output of stat(2), except atime, so that we don't have to read
/// them twice during a single build process.
pub fn file_hash(filename: &Path) -> Result<String> {
    const BUFSIZE: usize = 4096;
    let mut acc = Sha256::new();
    let mut f = File::open(filename).with_context(|| format!("Reading input file '{}'", filename.to_string_lossy()))?;
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

fn string_hash(s: &str) -> String {
    let mut acc = Sha256::new();
    acc.update(s.as_bytes());
    format!("{:x}", acc.finalize())
}

fn bytes_hash(s: &[u8]) -> String {
    let mut acc = Sha256::new();
    acc.update(s);
    format!("{:x}", acc.finalize())
}

/// Helper function for both input and output hash finalization.
fn bundle_hash<'a, I: Iterator<Item = (&'a str, &'a str)>>(hash_details: I) -> String {
    let mut acc: Sha256 = Sha256::new();
    for (tag, hash) in hash_details {
        acc.update(tag);
        acc.update(hash);
    }
    format!("{:x}", acc.finalize())
}

impl InputSet {
    /// Returns the HEX string of the hash of the whole input set.
    ///
    /// We calculate the whole hash bundle, and discard the separate hashes.
    pub fn hash(self, root: &Option<String>) -> Result<String> {
        self.hash_bundle(root).map(|x| x.hash)
    }

    /// Returns the HEX string of the hash of the files in the input set, and the total hash.
    ///
    /// It does this by calculating a SHA256 hash of all SHA256 hashes of inputs (being either file
    /// or tool tag) sorted by the values of the hashes themselves.
    pub fn hash_bundle(self, root: &Option<String>) -> Result<InputHashBundle> {
        // Calculate the hash of the input set independently of the order.
        let mut hash_bundle = InputHashBundle::default();
        for input in self.inputs {
            let hash = match input {
                Input::File(ref filename) => {
                    let path = filename.to_path(root)?;
                    file_hash(&path)?
                }
                Input::ToolTag(ref s) => string_hash(s),
            };
            hash_bundle.hash_details.push((input, hash));
        }
        // Sort inputs hashes by the hash value, but so that tool_tags come first.
        // This is needed so that when we cap our JSON, we could still see tool_tags.
        hash_bundle.hash_details.sort_by(|a, b| {
            if let Input::ToolTag(_) = a.0 {
                if let Input::ToolTag(_) = b.0 {
                    a.1.cmp(&b.1)
                } else {
                    Ordering::Less
                }
            } else {
                a.1.cmp(&b.1)
            }
        });
        hash_bundle.hash = bundle_hash(hash_bundle.hash_details.iter().map(|(inp, hash)| {
            (
                match inp {
                    Input::File(_) => "File",
                    Input::ToolTag(_) => "ToolTag",
                },
                &hash[..],
            )
        }));
        Ok(hash_bundle)
    }

    pub fn add_input(&mut self, input: Input) {
        self.inputs.push(input)
    }
}

impl OutputSet {
    /// Returns the HEX string of the hash of the whole input set.
    ///
    /// We calculate the whole hash bundle, and discard the separate hashes.
    pub fn hash(self, root: &Option<String>) -> Result<String> {
        self.hash_bundle(root).map(|x| x.hash)
    }

    /// Returns the HEX string of the hash of the files in the input set, and the total hash.
    ///
    /// It does this by calculating a SHA256 hash of all SHA256 hashes of inputs (being either file
    /// or tool tag) sorted by the values of the hashes themselves.
    pub fn hash_bundle(self, root: &Option<String>) -> Result<OutputHashBundle> {
        // Calculate the hash of the input set independently of the order.
        let mut hash_bundle = OutputHashBundle::default();
        for output in self.outputs {
            let hash = match output {
                Output::File(ref file_output) => {
                    if file_output.present {
                        let path = file_output.filename.to_path(root)?;
                        file_hash(&path)?
                    } else {
                        "".to_string()
                    }
                }
                Output::ExitCode(code) => string_hash(&code.to_string()),
                Output::Stdout(ref buffer) => bytes_hash(buffer),
                Output::Stderr(ref buffer) => bytes_hash(buffer),
            };
            hash_bundle.hash_details.push((output, hash));
        }
        // Sort inputs hashes by the hash value.
        hash_bundle.hash_details.sort_by(|a, b| a.1.cmp(&b.1));
        hash_bundle.hash = bundle_hash(hash_bundle.hash_details.iter().map(|(inp, hash)| {
            (
                match inp {
                    Output::File(_) => "File",
                    Output::ExitCode(_) => "ExitCode",
                    Output::Stdout(_) => "StdOut",
                    Output::Stderr(_) => "StdErr",
                },
                &hash[..],
            )
        }));
        Ok(hash_bundle)
    }

    pub fn add_output(&mut self, output: Output) {
        self.outputs.push(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    const EMPTY_SHA256: &'static str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn file_hash_test() -> Result<()> {
        let file = NamedTempFile::new()?;
        let hash = file_hash(file.path())?;
        // Sha256 hash of an empty file.
        assert_eq!(hash, EMPTY_SHA256);
        Ok(())
    }

    #[test]
    fn file_hash_nonexistent() {
        assert!(file_hash(Path::new("/nonexistent-capsule-input")).is_err());
    }

    #[test]
    fn test_input_set_empty() {
        let input_set = InputSet::default();
        assert_eq!(input_set.hash(&None).unwrap(), EMPTY_SHA256);
    }

    #[test]
    fn test_input_set_1() {
        let mut input_set = InputSet::default();
        let tool_tag = String::from("some tool_tag");
        input_set.add_input(Input::ToolTag(tool_tag));
        let hash1 = input_set.hash(&None).unwrap();
        assert_ne!(hash1, EMPTY_SHA256);
    }

    #[test]
    fn test_input_set_different_order() {
        let mut input_set1 = InputSet::default();
        let tool_tag1 = String::from("some tool_tag");
        let tool_tag2 = String::from("another tool_tag");
        input_set1.add_input(Input::ToolTag(tool_tag1.clone()));
        input_set1.add_input(Input::ToolTag(tool_tag2.clone()));
        let mut input_set2 = InputSet::default();
        input_set2.add_input(Input::ToolTag(tool_tag2));
        input_set2.add_input(Input::ToolTag(tool_tag1));
        assert_eq!(input_set1.hash(&None).unwrap(), input_set2.hash(&None).unwrap());
    }

    #[test]
    fn test_input_set_bundle() {
        let mut input_set1 = InputSet::default();
        let tool_tag1 = String::from("some tool_tag");
        let tool_tag2 = String::from("another tool_tag");
        input_set1.add_input(Input::ToolTag(tool_tag1.clone()));
        input_set1.add_input(Input::ToolTag(tool_tag2.clone()));
        let mut input_set2 = InputSet::default();
        input_set2.add_input(Input::ToolTag(tool_tag2));
        input_set2.add_input(Input::ToolTag(tool_tag1));
        let bundle1 = input_set1.hash_bundle(&None).unwrap();
        let bundle2 = input_set2.hash_bundle(&None).unwrap();
        assert_eq!(bundle1.hash, bundle2.hash);
        assert_eq!(bundle1.hash_details, bundle2.hash_details);
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
        input_set.add_input(Input::File(file1.path().into()));
        // These hashes were obtained by manual manipulation files and `openssl sha256`
        assert_eq!(
            input_set.clone().hash(&None).unwrap(),
            "f409e4c7ae76997e69556daae6139bee1f02e4f618d3da8deea10bb35b6c0ebd"
        );
        input_set.add_input(Input::File(file2.path().into()));
        assert_eq!(
            input_set.hash(&None).unwrap(),
            "a282f3da61a4bc322a8d31da6d30a0e924017962acbef2f6996b81709de8cdc3"
        );
    }
}
