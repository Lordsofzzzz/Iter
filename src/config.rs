use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

/// A single model entry from the TOML config.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelEntry {
    pub name: String,
    #[serde(default)]
    pub api: String,
    #[serde(default)]
    pub reasoning: bool,
    pub context_window: Option<u32>,
    pub max_tokens: Option<u32>,
    pub cost_input: Option<f64>,
    pub cost_output: Option<f64>,
    #[serde(default)]
    pub has_vision: bool,
}

/// Top-level config structure matching the TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    #[serde(rename = "model")]
    pub models: HashMap<String, ModelEntry>,
}

impl ModelConfig {
    /// Load the config: embedded defaults merged with user overrides.
    pub fn load() -> Self {
        let default_toml = include_str!("../default-models.toml");
        let mut config: ModelConfig =
            toml::from_str(default_toml).expect("invalid default-models.toml");

        if let Some(home) = dirs::home_dir() {
            let user_path = home.join(".iter").join("models.toml");
            if user_path.exists() {
                if let Ok(user_toml) = std::fs::read_to_string(&user_path) {
                    if let Ok(user_config) = toml::from_str::<ModelConfig>(&user_toml) {
                        config.models.extend(user_config.models);
                    }
                }
            }
        }

        config
    }

    /// Return the model list for the picker: (id, display_name, context_window_str).
    pub fn picker_entries(&self) -> Vec<(&str, &str, &str)> {
        struct Entry {
            id: String,
            name: String,
            ctx: String,
        }
        let mut entries: Vec<Entry> = self
            .models
            .iter()
            .map(|(id, m)| {
                let ctx = m
                    .context_window
                    .map(|c| format_ctx(c))
                    .unwrap_or_default();
                Entry {
                    id: id.clone(),
                    name: m.name.clone(),
                    ctx,
                }
            })
            .collect();
        // Sort by name for the picker
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        // We need 'static lifetimes for the picker — leak once
        let mut static_entries: Vec<(&str, &str, &str)> = Vec::with_capacity(entries.len());
        for e in &entries {
            let id = Box::leak(e.id.clone().into_boxed_str());
            let name = Box::leak(e.name.clone().into_boxed_str());
            let ctx = Box::leak(e.ctx.clone().into_boxed_str());
            static_entries.push((id, name, ctx));
        }
        static_entries
    }
}

fn format_ctx(ctx: u32) -> String {
    if ctx >= 1_000_000 {
        format!("{}M", ctx / 1_000_000)
    } else if ctx >= 1_000 {
        format!("{}k", ctx / 1_000)
    } else {
        ctx.to_string()
    }
}

/// Global model config, loaded once.
pub fn global_model_config() -> &'static ModelConfig {
    static CONFIG: OnceLock<ModelConfig> = OnceLock::new();
    CONFIG.get_or_init(ModelConfig::load)
}

/// Global model entries for the picker, loaded once.
pub fn global_model_entries() -> &'static [(&'static str, &'static str, &'static str)] {
    static ENTRIES: OnceLock<Vec<(&'static str, &'static str, &'static str)>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        let config = global_model_config();
        config.picker_entries()
    })
}
