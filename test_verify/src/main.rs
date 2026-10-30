fn main() {
    let v = "2026:10:22-12-00-00Z";
    println!("Testing: {}", v);
    match chrono::DateTime::parse_from_rfc3339(v) {
        Ok(dt) => println!("PARSED: {:?}", dt),
        Err(e) => println!("ERROR: {:?}", e),
    }
}