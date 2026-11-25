pub(super) fn parse_shape_geometry(raw: &str) -> Result<[i32;4], &'static str> {
    const ERROR: &str = "請依序輸入四個整數：X、Y、寬、高。";
    let mut fields = raw.split(|c: char| c == ',' || c == '，' || c.is_whitespace())
        .filter(|field| !field.is_empty());
    let mut values = [0; 4];
    for value in &mut values {
        *value = fields.next().ok_or(ERROR)?.parse().map_err(|_| ERROR)?;
    }
    if fields.next().is_some() { return Err(ERROR); }
    let [x,y,w,h] = values;
    if !(0..=10000).contains(&x) || !(0..=50000).contains(&y)
        || !(24..=2048).contains(&w) || !(24..=2048).contains(&h) {
        return Err("位置或尺寸超出範圍。X：0–10000，Y：0–50000，寬與高：24–2048。");
    }
    Ok(values)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_geometry_with_full_width_commas_and_keeps_bounds() {
        assert_eq!(parse_shape_geometry(" 450，140，120，60 "), Ok([450,140,120,60]));
        assert_eq!(parse_shape_geometry("0 50000 24 2048"), Ok([0,50000,24,2048]));
        for raw in ["", "1,1,120", "-1,2,120,60", "1,1,23,60", "1,1,120,60,8",
                    "1,1,2147483648,60", "1,1,120px,60"] {
            assert!(parse_shape_geometry(raw).is_err(), "{raw}");
        }
    }
}
