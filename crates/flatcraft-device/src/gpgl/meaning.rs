//! Meaning texts shared by the job listing (`GpglCommand::describe`) and the decoder, so both
//! always say the same thing (gpgl.md section 3).

pub const INIT: &str = "init";
pub const STATUS_QUERY: &str = "status query";
pub const FIRMWARE_QUERY: &str = "firmware version query";
pub const ORIENTATION_PORTRAIT: &str = "orientation portrait";
pub const LINE_SOLID: &str = "line type solid";
pub const HOME: &str = "home";
pub const UNKNOWN: &str = "unknown";

pub fn speed(v: i64) -> String {
    format!("speed {v}")
}

pub fn force(v: i64) -> String {
    format!("force {v}")
}

pub fn blade_offset(fc: i64) -> String {
    if fc == 0 {
        "blade offset 0 (no firmware compensation)".into()
    } else {
        format!("blade offset {fc} (firmware compensation)")
    }
}

pub fn track_enhancing(on: bool) -> &'static str {
    if on {
        "track enhancing on"
    } else {
        "track enhancing off"
    }
}

pub fn tb50(landscape: bool) -> &'static str {
    if landscape {
        "orientation landscape (TB50)"
    } else {
        "orientation portrait (TB50)"
    }
}

pub fn work_area_upper_left(y: i64, x: i64) -> String {
    format!("work area upper-left y={y}, x={x}")
}

pub fn work_area_lower_right(y: i64, x: i64) -> String {
    format!("work area lower-right y={y}, x={x}")
}

pub fn lift(values: &[i64]) -> String {
    let list: Vec<String> = values.iter().map(i64::to_string).collect();
    format!("lift setting {}", list.join(","))
}

pub fn corner(start: i64, end: i64, tool: i64) -> String {
    format!("corner sharpening {start},{end},{tool}")
}

pub fn move_to(y: i64, x: i64) -> String {
    format!("move to y={y}, x={x}")
}

pub fn draw(points: usize) -> String {
    if points == 1 {
        "draw through 1 point".into()
    } else {
        format!("draw through {points} points")
    }
}

pub fn forbidden(what: &str) -> String {
    format!("FORBIDDEN: calibration write ({what}), never send")
}

#[cfg(test)]
mod tests {
    #[test]
    fn one_point_is_singular() {
        assert_eq!(super::draw(1), "draw through 1 point");
        assert_eq!(super::draw(2), "draw through 2 points");
    }
}
