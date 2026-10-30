fn main() {
    let v = "2026:10:22-12-00-00Z";
    println!("{:?}", chrono::DateTime::parse_from_rfc3339(v));
}