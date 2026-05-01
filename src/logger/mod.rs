//! Structured terminal logging.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    Planner,
    Coder,
    Reviewer,
}

impl AgentRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planner => "Planner",
            Self::Coder => "Coder",
            Self::Reviewer => "Reviewer",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Logger {
    verbose: bool,
}

impl Logger {
    pub fn new(verbose: bool) -> Self {
        Self { verbose }
    }

    pub fn log(&self, role: AgentRole, message: &str) {
        self.log_with_prefix(role, "", message);
    }

    pub fn log_with_prefix(&self, role: AgentRole, prefix: &str, message: &str) {
        let role_str = role.as_str();
        if prefix.is_empty() {
            println!("[{}] → {}", role_str, message);
        } else {
            println!("[{}] {}: {}", role_str, prefix, message);
        }
    }

    pub fn info(&self, message: &str) {
        println!("[INFO] {}", message);
    }

    pub fn error(&self, message: &str) {
        eprintln!("[ERROR] {}", message);
    }

    pub fn warn(&self, message: &str) {
        println!("[WARN] {}", message);
    }

    pub fn success(&self, message: &str) {
        println!("[SUCCESS] {}", message);
    }

    pub fn is_verbose(&self) -> bool {
        self.verbose
    }
}

impl Default for Logger {
    fn default() -> Self {
        Self::new(false)
    }
}
