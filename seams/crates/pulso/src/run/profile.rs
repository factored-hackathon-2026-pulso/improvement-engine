//! `PULSO_PROFILE`: the SUPPORT-floor profile of the loop sensor (R4). `standard` (the default, what an unset variable means) or `demo`.
//!
//! `demo` lowers the cell support floors (`steps::cells::Config::demo`: strict 500 -> 100, exploratory 200 -> 60) and raises the number of
//! exploratory findings that join a run (2 -> 5) so a planted effect on small SYNTHETIC cells reaches the roles. It never touches the
//! privacy floor (`k_min` stays 10), never the statistical levels, and it is refused unless it is set explicitly AND the cells are
//! declared synthetic (`PULSO_CELLS_SOURCE=synthetic`): real or bank data can never run with lowered floors. Every record the loop
//! writes carries `support_profile`.
use reasoning::finding::Source;
use steps::cells::{Config, K_FLOOR};

/// Exploratory findings per run under the demo profile (standard: 2). `PULSO_LOOP_MAX_EXPLORATORY` still wins when set.
pub const DEMO_MAX_EXPLORATORY: usize = 5;
pub const STANDARD_MAX_EXPLORATORY: usize = 2;

/// Which cells table the loop reads: the bank/E0 metrics (M*, E*) or the platform event families (P_*, EVT1/EVT2).
/// PULSO_CELLS_FAMILY=platform selects the platform sensor profile (steps::cells::Config::platform).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Bank,
    Platform,
}

impl Family {
    pub fn as_str(self) -> &'static str {
        match self {
            Family::Bank => "bank",
            Family::Platform => "platform",
        }
    }
    pub fn from_lookup(get: &dyn Fn(&str) -> Option<String>) -> Result<Family, String> {
        match get("PULSO_CELLS_FAMILY").unwrap_or_default().trim() {
            "" | "bank" => Ok(Family::Bank),
            "platform" => Ok(Family::Platform),
            _ => Err("PULSO_CELLS_FAMILY is not bank|platform".into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Standard,
    Demo,
}

impl Profile {
    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Standard => "standard",
            Profile::Demo => "demo",
        }
    }

    pub fn from_lookup(get: &dyn Fn(&str) -> Option<String>, source: Source) -> Result<Profile, String> {
        let raw = get("PULSO_PROFILE").unwrap_or_default();
        match raw.trim() {
            "" | "standard" => Ok(Profile::Standard),
            "demo" if source == Source::Synthetic => Ok(Profile::Demo),
            "demo" => Err("PULSO_PROFILE=demo is refused: it lowers the support floors and is only allowed with PULSO_CELLS_SOURCE=synthetic".into()),
            _ => Err("PULSO_PROFILE is not standard|demo".into()),
        }
    }

    /// The sensor configuration. `Standard` keeps the existing choice (exploratory only when the loop admits exploratory findings).
    pub fn sensor_config(self, exploratory_findings: usize) -> Config {
        let cfg = match self {
            Profile::Demo => Config::demo(),
            Profile::Standard if exploratory_findings > 0 => Config::exploratory(),
            Profile::Standard => Config::default(),
        };
        assert!(cfg.k_min >= K_FLOOR, "no profile may lower the privacy floor");
        cfg
    }

    /// The platform-event sensor: pooled support 200, the registered level risk, same-channel baselines. Demo lowers the support floor
    /// to the demo one (synthetic data only, enforced by from_lookup); the privacy floor and the statistical levels never change.
    pub fn platform_config(self) -> Config {
        let mut cfg = Config::platform();
        if self == Profile::Demo {
            cfg.min_support = steps::cells::DEMO_MIN_SUPPORT;
            cfg.support_profile = "demo";
        }
        assert!(cfg.k_min >= K_FLOOR, "no profile may lower the privacy floor");
        cfg
    }

    pub fn sensor_config_for(self, family: Family, exploratory_findings: usize) -> Config {
        match family {
            Family::Bank => self.sensor_config(exploratory_findings),
            Family::Platform => self.platform_config(),
        }
    }

    pub fn default_max_exploratory(self) -> usize {
        match self {
            Profile::Demo => DEMO_MAX_EXPLORATORY,
            Profile::Standard => STANDARD_MAX_EXPLORATORY,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lk(v: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> + use<> {
        move |k| v.iter().find(|(n, _)| *n == k).map(|(_, x)| x.to_string())
    }

    #[test]
    fn unset_or_standard_is_the_standard_profile() {
        assert_eq!(Profile::from_lookup(&lk(&[]), Source::BankTreated), Ok(Profile::Standard));
        assert_eq!(Profile::from_lookup(&lk(&[("PULSO_PROFILE", "standard")]), Source::E0Treated), Ok(Profile::Standard));
        assert_eq!(Profile::from_lookup(&lk(&[("PULSO_PROFILE", "")]), Source::BankTreated), Ok(Profile::Standard));
    }

    #[test]
    fn demo_is_only_ever_explicit_and_only_for_synthetic_data() {
        assert_eq!(Profile::from_lookup(&lk(&[("PULSO_PROFILE", "demo")]), Source::Synthetic), Ok(Profile::Demo));
        for s in [Source::BankTreated, Source::E0Treated] {
            let e = Profile::from_lookup(&lk(&[("PULSO_PROFILE", "demo")]), s).unwrap_err();
            assert!(e.contains("PULSO_PROFILE=demo") && e.contains("synthetic"), "{e}");
        }
    }

    #[test]
    fn an_unknown_profile_is_refused_not_ignored() {
        assert_eq!(Profile::from_lookup(&lk(&[("PULSO_PROFILE", "demo ")]), Source::Synthetic), Ok(Profile::Demo), "surrounding blanks are trimmed");
        assert!(Profile::from_lookup(&lk(&[("PULSO_PROFILE", "fast")]), Source::Synthetic).unwrap_err().contains("standard|demo"));
        assert!(Profile::from_lookup(&lk(&[("PULSO_PROFILE", "DEMO")]), Source::Synthetic).is_err());
    }

    #[test]
    fn the_demo_floors_are_lower_but_the_privacy_floor_and_the_levels_are_not() {
        let (d, s) = (Profile::Demo.sensor_config(5), Profile::Standard.sensor_config(2));
        assert!(d.min_support < s.min_support);
        assert!(d.exploratory.as_ref().unwrap().min_support < s.exploratory.as_ref().unwrap().min_support);
        assert_eq!(d.k_min, s.k_min);
        assert!(d.k_min >= 10);
        assert_eq!((d.alpha, d.min_effect, d.min_ratio), (s.alpha, s.min_effect, s.min_ratio));
        assert_eq!(d.support_profile, "demo");
        assert_eq!(s.support_profile, "standard");
    }

    #[test]
    fn the_platform_family_selects_the_platform_sensor_and_keeps_the_floors() {
        assert_eq!(Family::from_lookup(&lk(&[])), Ok(Family::Bank));
        assert_eq!(Family::from_lookup(&lk(&[("PULSO_CELLS_FAMILY", "platform")])), Ok(Family::Platform));
        assert!(Family::from_lookup(&lk(&[("PULSO_CELLS_FAMILY", "plat")])).is_err());
        let (p, d) = (Profile::Standard.sensor_config_for(Family::Platform, 2), Profile::Demo.sensor_config_for(Family::Platform, 5));
        assert!(p.platform && d.platform && p.level_risks.len() == 1);
        assert_eq!((p.min_support, d.min_support), (200, 100));
        assert_eq!((p.k_min, p.alpha), (d.k_min, d.alpha));
        assert!(!Profile::Standard.sensor_config_for(Family::Bank, 0).platform);
    }

    #[test]
    fn standard_keeps_the_strict_sensor_when_no_exploratory_findings_are_admitted() {
        assert!(Profile::Standard.sensor_config(0).exploratory.is_none());
        assert!(Profile::Standard.sensor_config(2).exploratory.is_some());
        assert!(Profile::Demo.default_max_exploratory() > Profile::Standard.default_max_exploratory());
    }
}
