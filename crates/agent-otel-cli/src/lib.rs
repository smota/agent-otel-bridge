/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

//! # agent-otel-bridge
//!
//! Ultra-fast, zero-overhead OpenTelemetry instrumentation bridge for AI CLI agent harnesses
//! (Google Antigravity, Claude Code, OpenAI Codex, xAI Grok, Pi [pi.dev]).
//!
//! This crate provides the unified CLI and daemon bridge, delegating core types to [`agent_otel_core`]
//! and IPC utilities to [`agent_otel_ipc`].

pub mod doctor;
pub mod emit_quota;
pub mod hooks;
pub mod local;
pub mod scanner;
pub mod stop;

pub use agent_otel_core as core;
pub use agent_otel_daemon as daemon;
pub use agent_otel_ipc as ipc;
