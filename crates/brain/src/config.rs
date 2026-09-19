//! Profiles and tiers, loaded from `config/*.yaml`. No secrets live here: a
//! profile's OpenRouter key is the env var named by [`Profile::key_env`].

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub tier: String,
    pub daily_limit_usd: f64,
    pub monthly_soft_usd: f64,
    #[serde(default = "default_l2")]
    pub l2_cache: String,
}

fn default_l2() -> String {
    "off".into()
}

impl Profile {
    /// `dev` → `OPENROUTER_KEY_DEV`
    pub fn key_env(&self) -> String {
        format!("OPENROUTER_KEY_{}", self.name.to_ascii_uppercase())
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Tier {
    pub model: Option<String>,
    pub fallback: Option<String>,
    #[serde(default)]
    pub context: u64,
    #[serde(default)]
    pub input_usd_per_m: f64,
    #[serde(default)]
    pub output_usd_per_m: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct ProfilesFile {
    profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Deserialize)]
struct TiersFile {
    tiers: BTreeMap<String, Tier>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub profiles: Vec<Profile>,
    pub tiers: BTreeMap<String, Tier>,
}

impl Config {
    pub fn load(dir: &Path) -> Result<Self> {
        let profiles: ProfilesFile = read_yaml(&dir.join("profiles.yaml"))?;
        let tiers: TiersFile = read_yaml(&dir.join("tiers.yaml"))?;
        let cfg = Config {
            profiles: profiles.profiles,
            tiers: tiers.tiers,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    #[cfg(test)]
    pub fn from_yaml(profiles: &str, tiers: &str) -> Result<Self> {
        let p: ProfilesFile = serde_yaml::from_str(profiles)?;
        let t: TiersFile = serde_yaml::from_str(tiers)?;
        let cfg = Config {
            profiles: p.profiles,
            tiers: t.tiers,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for p in &self.profiles {
            if !seen.insert(p.name.as_str()) {
                bail!("duplicate profile `{}`", p.name);
            }
            if !p.name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                bail!("profile name `{}` must be lowercase [a-z_]", p.name);
            }
            if !self.tiers.contains_key(&p.tier) {
                bail!("profile `{}` references unknown tier `{}`", p.name, p.tier);
            }
            if p.daily_limit_usd <= 0.0 {
                bail!("profile `{}` needs a positive daily_limit_usd", p.name);
            }
        }
        Ok(())
    }

    pub fn profile(&self, name: &str) -> Result<&Profile> {
        self.profiles
            .iter()
            .find(|p| p.name == name)
            .with_context(|| format!("unknown profile `{name}`"))
    }

    /// The model to send to OpenRouter for a tier.
    pub fn model_for_tier(&self, tier: &str) -> Result<&str> {
        self.tiers
            .get(tier)
            .and_then(|t| t.model.as_deref())
            .with_context(|| format!("tier `{tier}` has no model configured"))
    }
}

fn read_yaml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_yaml::from_str(&text).with_context(|| format!("invalid YAML in {}", path.display()))
}

/// Finds the `config/` directory: `--config`, then upwards from the cwd.
pub fn find_config_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    let mut dir = std::env::current_dir()?;
    loop {
        let candidate = dir.join("config");
        if candidate.join("profiles.yaml").is_file() {
            return Ok(candidate);
        }
        if !dir.pop() {
            bail!(
                "no config/profiles.yaml found upwards from the current directory; pass --config"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILES: &str = r#"
profiles:
  - name: dev
    tier: fast
    daily_limit_usd: 3.0
    monthly_soft_usd: 30.0
  - name: car
    tier: fast
    daily_limit_usd: 0.2
    monthly_soft_usd: 3.0
    l2_cache: exact
"#;
    const TIERS: &str = r#"
tiers:
  fast:
    model: prism-ml/ternary-bonsai-2-27b
    fallback: deepseek/deepseek-v4-flash
    context: 262144
    input_usd_per_m: 0.075
    output_usd_per_m: 0.5
  premium:
    model: null
"#;

    #[test]
    fn loads_profiles_and_tiers() {
        let cfg = Config::from_yaml(PROFILES, TIERS).unwrap();
        assert_eq!(cfg.profiles.len(), 2);
        assert_eq!(cfg.profile("dev").unwrap().key_env(), "OPENROUTER_KEY_DEV");
        assert_eq!(cfg.profile("car").unwrap().l2_cache, "exact");
        assert_eq!(cfg.profile("dev").unwrap().l2_cache, "off");
        assert_eq!(
            cfg.model_for_tier("fast").unwrap(),
            "prism-ml/ternary-bonsai-2-27b"
        );
        assert!(cfg.model_for_tier("premium").is_err());
    }

    #[test]
    fn rejects_unknown_tier_and_duplicates() {
        let bad_tier = PROFILES.replace(
            "tier: fast\n    daily_limit_usd: 0.2",
            "tier: nope\n    daily_limit_usd: 0.2",
        );
        assert!(Config::from_yaml(&bad_tier, TIERS).is_err());
        let dup = PROFILES.replace("name: car", "name: dev");
        assert!(Config::from_yaml(&dup, TIERS).is_err());
    }

    #[test]
    fn repo_config_files_are_valid() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
        let cfg = Config::load(&dir).unwrap();
        for name in ["dev", "ago", "benchmark", "assistant", "car"] {
            assert!(cfg.profile(name).is_ok(), "profile {name}");
        }
        assert!(cfg.model_for_tier("fast").is_ok());
        assert!(cfg.model_for_tier("reasoning").is_ok());
    }
}
