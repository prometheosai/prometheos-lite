# Repo Workbench Demo Transcript

This transcript shows the intended alpha demo path.

## 1. Create a WorkContext

```bash
prometheos work create \
  --repo fixtures/repo-workbench/rust-risky \
  --goal "Find risky code and suggest safe improvements" \
  --mode review \
  --json
```

Example output:

```json
{
  "work_id": "<work_id>",
  "repo": "fixtures/repo-workbench/rust-risky",
  "status": "created",
  "next": "prometheos work run <work_id>"
}
```

## 2. Run the WorkContext

```bash
prometheos work run <work_id>
```

Expected result:

- repository scanned
- risk report generated
- suggested patch plan generated
- memory written
- no source files modified

## 3. Inspect artifacts

```bash
prometheos work artifacts <work_id>
```

Expected artifacts:

- risk report
- suggested patch plan

## 4. Show memory

```bash
prometheos work memory show <work_id>
```

Expected result:

- WorkContext summary
- goal
- repo path
- status
- artifact references
- next action

## 5. Continue

```bash
prometheos work continue <work_id>
```

Expected result:

- previous context restored
- next action displayed
