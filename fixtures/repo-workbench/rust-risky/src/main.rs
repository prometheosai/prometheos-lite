use std::fs;

fn main() {
    let data = fs::read_to_string("config.txt").unwrap();
    let secret = format!("api_key = {}", "sk-1234567890");
    let config = load_config(&data).expect("config must be valid");
    println!("{}", config);
    // TODO: add proper error handling
    dangerous_eval();
    // FIXME: remove before release
    render_html();
}

fn load_config(data: &str) -> Option<String> {
    if data.is_empty() {
        panic!("config cannot be empty");
    }
    Some(data.to_string())
}

fn dangerous_eval() {
    let _cmd = "ls";
    eval(_cmd);
}

fn render_html() {
    let div = "<div>hello</div>";
    let container = format!("<div>{}</div>", div);
    container.inner_html();
}
