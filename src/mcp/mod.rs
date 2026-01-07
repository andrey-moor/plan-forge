//! MCP Server module for exposing plan-forge functionality to AI assistants.
//!
//! This module provides an MCP (Model Context Protocol) server that exposes
//! planning tools for integration with tools like Claude Code, Cursor, and VS Code.

pub mod server;
pub mod status;

pub use server::PlanForgeServer;
pub use status::SessionStatus;
