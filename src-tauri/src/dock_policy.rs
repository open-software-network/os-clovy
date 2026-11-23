#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockPolicy {
    Regular,
    Accessory,
}

pub fn desired_policy(hide_from_dock: bool, window_visible: bool) -> DockPolicy {
    if hide_from_dock && !window_visible {
        DockPolicy::Accessory
    } else {
        DockPolicy::Regular
    }
}

#[cfg(test)]
mod dock_policy_tests {
    use super::*;
    #[test]
    fn policy_covers_visibility_and_preference() {
        assert_eq!(desired_policy(false, false), DockPolicy::Regular);
        assert_eq!(desired_policy(false, true), DockPolicy::Regular);
        assert_eq!(desired_policy(true, true), DockPolicy::Regular);
        assert_eq!(desired_policy(true, false), DockPolicy::Accessory);
    }
}
