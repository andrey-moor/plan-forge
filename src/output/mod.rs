pub mod files;

pub use files::*;

use anyhow::Result;
use async_trait::async_trait;

use crate::models::{Plan, ReviewResult};

/// Trait for writing plan outputs
#[async_trait]
pub trait OutputWriter: Send + Sync {
    /// Write intermediate plan output during iteration
    async fn write_intermediate(&self, plan: &Plan, iteration: u32) -> Result<()>;

    /// Write review output
    async fn write_review(&self, review: &ReviewResult, iteration: u32) -> Result<()>;

    /// Write final plan output
    async fn write_final(&self, plan: &Plan) -> Result<()>;
}
