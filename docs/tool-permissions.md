# Tool Permissions System

## Overview

The tool permissions system provides declarative control over what tools can be executed. It separates **WHAT** is allowed (ToolPermission/ToolPolicy) from **HOW** it is enforced (ToolSandboxProfile).

## Permission Types

### ToolPermission Enum

```rust
use prometheos_lite::tools::ToolPermission;

pub enum ToolPermission {
    Network,    // HTTP requests, network access
    FileRead,   // File read operations
    FileWrite,  // File write operations
    Shell,      // Shell command execution
    Env,        // Environment variable access
}
```

## ToolPolicy

### Structure

```rust
use prometheos_lite::tools::ToolPolicy;

pub struct ToolPolicy {
    pub allowed_permissions: HashSet<ToolPermission>,
    pub require_approval: bool,
    pub restricted_write_paths: Vec<String>,
}
```

### Creating Policies

#### Conservative Policy (Default)

Safe defaults for production use:

```rust
let policy = ToolPolicy::conservative();
// Allows: FileRead only
// Restricts: FileWrite to prometheos-output/
// Denies: Network, Shell, Env
```

#### Permissive Policy

For development/testing:

```rust
let policy = ToolPolicy::permissive();
// Allows: All permissions
```

#### Custom Policy

```rust
let policy = ToolPolicy::new()
    .with_permission(ToolPermission::FileRead)
    .with_permission(ToolPermission::Network)
    .with_approval(true)
    .with_restricted_write_path("safe-dir/".to_string());
```

### Checking Permissions

```rust
if policy.is_allowed(ToolPermission::Network) {
    // Network access is allowed
}
```

## ToolSandboxProfile Integration

### Declarative vs Runtime

- **ToolPermission/ToolPolicy**: Declarative (WHAT is allowed)
- **ToolSandboxProfile**: Runtime enforcement (HOW it is enforced)

### Integration

```rust
use prometheos_lite::flow::intelligence::ToolSandboxProfile;
use prometheos_lite::tools::ToolPolicy;

let tool_policy = ToolPolicy::conservative();
let profile = ToolSandboxProfile::with_tool_policy(tool_policy);
```

The `ToolSandboxProfile` uses the `ToolPolicy` to set runtime flags:
- `allow_network` = `policy.is_allowed(Network)`
- `allow_file_read` = `policy.is_allowed(FileRead)`
- `allow_file_write` = `policy.is_allowed(FileWrite)`

### Shell Permission Check

Before executing shell commands:

```rust
if !profile.tool_policy.is_allowed(ToolPermission::Shell) {
    anyhow::bail!("Shell execution is not allowed by tool policy");
}
```

## Conservative Defaults

### Default Behavior

By default, PrometheOS Lite uses conservative defaults:

- **Shell**: Disabled (no command execution)
- **Network**: Denied (no HTTP requests)
- **File Write**: Restricted to `prometheos-output/` directory
- **File Read**: Allowed (with path restrictions)
- **Environment**: Denied (no env variable access)

### Path Restrictions

Blocked paths for file operations:
- `/etc` (Unix)
- `/sys` (Unix)
- `/proc` (Unix)

Allowed commands (read-only):
- `echo`, `cat`, `ls`, `pwd`, `grep`, `find`, `head`, `tail`, `wc`, `sort`, `uniq`, `cut`, `sed`, `awk`, `tr`, `diff`, `file`, `stat`, `date`, `whoami`, `hostname`, `uname`, `env`, `printenv`
- `cmd`, `powershell` (Windows)

Blocked commands (destructive):
- `rm`, `rmdir`, `mv`, `cp`, `dd`, `mkfs`, `fdisk`
- `format`, `del` (Windows)

## Tool Metadata

### ToolMetadata Structure

```rust
use prometheos_lite::tools::ToolMetadata;

pub struct ToolMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub schema_hash: String,
    pub version: String,
    pub metadata: HashMap<String, serde_json::Value>,
}
```

### Schema Hash

Generate a hash for tool schema validation:

```rust
let schema = serde_json::json!({
    "type": "object",
    "properties": {
        "path": {"type": "string"}
    }
});
let hash = ToolMetadata::generate_schema_hash(&schema);
```

## Usage Examples

### Creating a Safe Tool Runtime

```rust
use prometheos_lite::flow::intelligence::ToolRuntime;
use prometheos_lite::tools::ToolPolicy;

let tool_policy = ToolPolicy::conservative();
let profile = ToolSandboxProfile::with_tool_policy(tool_policy);
let tool_runtime = ToolRuntime::new(profile);
```

### Custom Policy for Specific Flow

```rust
let custom_policy = ToolPolicy::new()
    .with_permission(ToolPermission::FileRead)
    .with_permission(ToolPermission::Network)
    .with_restricted_write_path("workspace/".to_string());

let profile = ToolSandboxProfile::with_tool_policy(custom_policy);
```

### Permission Check Before Tool Execution

```rust
if !profile.tool_policy.is_allowed(ToolPermission::Shell) {
    return Err(anyhow::anyhow!("Shell execution not allowed"));
}

let output = tool_runtime.execute_command("ls", vec!["-la"]).await?;
```

## Best Practices

1. **Use Conservative Defaults**: Start with conservative policy, only add permissions as needed
2. **Restrict File Writes**: Always restrict file writes to specific directories
3. **Disable Shell in Production**: Shell execution should be disabled unless explicitly needed
4. **Audit Permissions**: Review allowed permissions regularly
5. **Log Permission Denials**: Log when tool execution is denied for debugging
6. **Document Policy**: Document why specific permissions are needed for each flow

## Security Considerations

- **Network Access**: Should be explicitly enabled and restricted to specific hosts
- **File System**: Restrict reads and writes to specific directories
- **Shell Execution**: High risk, should require explicit approval
- **Environment Access**: Should be disabled unless needed for specific tools
- **Approval Workflow**: Consider requiring user approval for sensitive operations
