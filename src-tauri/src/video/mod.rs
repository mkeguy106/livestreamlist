//! Inline-video session management (Phase 6 slice 2).
//!
//! One streamlink child per playing channel serving MPEG-TS over a localhost
//! port; a single CORS passthrough bridges those ports to the webview. See
//! docs/superpowers/specs/2026-07-08-inline-video-slice2-design.md.

#![allow(dead_code)] // removed in the manager task

pub(crate) mod session;
pub(crate) mod spawn;
