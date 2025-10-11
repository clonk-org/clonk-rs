pub(crate) fn integer_distance(x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    let dx = i64::from(x1) - i64::from(x2);
    let dy = i64::from(y1) - i64::from(y2);
    let d2 = dx * dx + dy * dy;
    if d2 < 0 {
        return -1;
    }
    let mut dist = (d2 as f64).sqrt() as i32;
    if i64::from(dist) * i64::from(dist) < d2 {
        dist += 1;
    }
    if i64::from(dist) * i64::from(dist) > d2 {
        dist -= 1;
    }
    dist
}
