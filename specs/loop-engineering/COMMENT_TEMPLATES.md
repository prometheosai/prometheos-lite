# Comment Templates

Standardized GitHub comment templates for loop-engineering operations.

## Progress update

```
**Loop Progress Update**

Mode: {Task Mode | Epic Completion Mode}
Current task: {task name}
Completed: {N} of {M} tasks
Blockers: {none or description}
Verification: {passed / partial / failed}
Next: {next task or handoff}
```

## Blocker report

```
**Blocker — execution stopped**

Stop reason: {blocker description}
Completed tasks: {list}
PR: {link if any}
Handoff: {link to handoff file}

This needs human review before continuing.
```

## Handoff notification

```
**Handoff**

Task: {task name}
Mode: {Task Mode | Epic Completion Mode}
Status: {completed / blocked / partial}
Verification: {summary}
Blockers: {none or description}
PR: {link if any}

See handoff file for full details.
```

## Verification failure

```
**Verification Failed**

Command: {command that failed}
Exit code: {exit code}
Output: {relevant output or link}

Task cannot proceed until this is resolved.
```

## Scope warning

```
**Scope Warning**

The following change appears to exceed the approved scope:

{description of change}

Proceeding would require explicit approval. Stopping per protocol.
```
