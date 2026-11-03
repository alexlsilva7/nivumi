//! Presentation values for the compact hydration panel.
pub fn liters(ml: u32, english: bool) -> String {
    let text = format!("{:.1}", ml as f64 / 1000.);
    if english {
        text
    } else {
        text.replace('.', ",")
    }
}
pub fn percentage(total: u32, goal: u32) -> u64 {
    (u64::from(total) * 100 + u64::from(goal.max(1)) / 2) / u64::from(goal.max(1))
}
pub fn chart_max(hours: &[u32; 24]) -> u64 {
    let peak = u64::from(hours.iter().copied().max().unwrap_or(0)).max(400);
    peak.div_ceil(200) * 200
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summary_keeps_excess_and_localizes_units() {
        assert_eq!(liters(5200, false), "5,2");
        assert_eq!(liters(5200, true), "5.2");
        assert_eq!(percentage(5200, 2000), 260);
        assert_eq!(percentage(0, 2000), 0);
    }
    #[test]
    fn chart_scale_includes_consumption_outside_reminder_hours() {
        let mut hours = [0; 24];
        assert_eq!(chart_max(&hours), 400);
        hours[23] = 1250;
        assert_eq!(chart_max(&hours), 1400);
        hours[0] = u32::MAX;
        assert!(chart_max(&hours) >= u64::from(u32::MAX));
    }
}
