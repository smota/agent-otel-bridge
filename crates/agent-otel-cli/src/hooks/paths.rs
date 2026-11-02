/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

use std::path::{Path, PathBuf};

pub fn get_antigravity_project_path(base: Option<&Path>) -> Option<PathBuf> {
    let root = base.unwrap_or_else(|| Path::new("."));
    Some(root.join(".gemini").join("hooks.json"))
}

pub fn get_antigravity_config_path(project: bool) -> Option<PathBuf> {
    if project {
        get_antigravity_project_path(None)
    } else {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|home| {
                PathBuf::from(home)
                    .join(".gemini")
                    .join("config")
                    .join("hooks.json")
            })
    }
}

pub fn get_claude_project_path(base: Option<&Path>) -> Option<PathBuf> {
    let root = base.unwrap_or_else(|| Path::new("."));
    Some(root.join(".claude").join("settings.json"))
}

pub fn get_claude_config_path(project: bool) -> Option<PathBuf> {
    if project {
        get_claude_project_path(None)
    } else {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|home| PathBuf::from(home).join(".claude").join("settings.json"))
    }
}

pub fn get_codex_project_path(base: Option<&Path>) -> Option<PathBuf> {
    let root = base.unwrap_or_else(|| Path::new("."));
    Some(root.join(".codex").join("hooks.json"))
}

pub fn get_codex_config_path(project: bool) -> Option<PathBuf> {
    if project {
        get_codex_project_path(None)
    } else {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|home| PathBuf::from(home).join(".codex").join("hooks.json"))
    }
}

pub fn get_grok_project_path(base: Option<&Path>) -> Option<PathBuf> {
    let root = base.unwrap_or_else(|| Path::new("."));
    Some(root.join(".grok").join("hooks").join("agent-otel.json"))
}

pub fn get_grok_config_path(project: bool) -> Option<PathBuf> {
    if project {
        get_grok_project_path(None)
    } else {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|home| {
                PathBuf::from(home)
                    .join(".grok")
                    .join("hooks")
                    .join("agent-otel.json")
            })
    }
}

pub fn get_pi_project_path(base: Option<&Path>) -> Option<PathBuf> {
    let root = base.unwrap_or_else(|| Path::new("."));
    Some(root.join(".pi").join("hooks.json"))
}

pub fn get_pi_config_path(project: bool) -> Option<PathBuf> {
    if project {
        get_pi_project_path(None)
    } else {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|home| PathBuf::from(home).join(".pi").join("hooks.json"))
    }
}
