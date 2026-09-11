/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

pub mod frame;
#[cfg(all(windows, feature = "client-sync"))]
pub mod client;
#[cfg(all(windows, feature = "server-async"))]
pub mod server;
