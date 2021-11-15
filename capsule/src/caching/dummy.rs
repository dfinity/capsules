use crate::caching::backend::CachingBackend;
use anyhow::Result;
use async_trait::async_trait;

use crate::iohashing::{HashBundle, OutputHashBundle};

#[derive(Default)]
pub struct DummyBackend {
    pub verbose_output: bool,
    pub capsule_id: String,
}

#[async_trait]
impl CachingBackend for DummyBackend {
    fn name(&self) -> &'static str {
        "dummy"
    }

    #[allow(unused_variables)]
    async fn write(&self, inputs_bundle: &HashBundle, output_bundle: &OutputHashBundle) -> Result<()> {
        println!(
            "Capsule ID: '{}'. Inputs key: '{}'",
            self.capsule_id,
            inputs_bundle.hash
        );
        if self.verbose_output {
            println!("  Capsule Inputs hashes: {:?}", inputs_bundle.hash_details);
        }
        Ok(())
    }
}
