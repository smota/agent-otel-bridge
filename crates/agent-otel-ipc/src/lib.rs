/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

#[cfg(feature = "client-sync")]
pub mod client;
pub mod frame;
#[cfg(feature = "server-async")]
pub mod server;
