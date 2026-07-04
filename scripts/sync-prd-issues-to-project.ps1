param(
    [Parameter(Mandatory = $true)]
    [string]$GithubToken,
    [string]$Owner = "prometheosai",
    [string]$Repo = "prometheos-lite",
    [int]$ProjectNumber = 3
)

$ErrorActionPreference = "Stop"

$headers = @{
    Authorization = "Bearer $GithubToken"
    "User-Agent" = "prometheos-lite-issue-sync"
    Accept = "application/vnd.github+json"
}

function Invoke-GithubGraphQL {
    param(
        [string]$Query,
        [hashtable]$Variables
    )

    $body = @{
        query     = $Query
        variables = $Variables
    } | ConvertTo-Json -Depth 20

    $response = Invoke-RestMethod -Method Post -Uri "https://api.github.com/graphql" -Headers $headers -Body $body
    if ($response.errors) {
        throw ("GraphQL error: " + ($response.errors | ConvertTo-Json -Depth 20))
    }

    return $response.data
}

function Invoke-GithubRest {
    param(
        [string]$Method,
        [string]$Uri,
        [object]$Body
    )

    if ($null -eq $Body) {
        return Invoke-RestMethod -Method $Method -Uri $Uri -Headers $headers
    }

    $json = $Body | ConvertTo-Json -Depth 20
    return Invoke-RestMethod -Method $Method -Uri $Uri -Headers $headers -Body $json
}

$issues = @(
    @{
        Title = "Initialize Rust workspace and CLI entrypoint"
        Body  = @"
## Goal
Create a working CLI entrypoint.

## Tasks
- Initialize Rust project (`cargo new prometheos-lite`)
- Add CLI parsing with `clap`
- Create base command `prometheos run ""<task>""`

## Suggested PR
`feat(cli): initialize Rust CLI with run command`
"@
        Labels = @("phase:0", "type:feature")
    },
    @{
        Title = "Scaffold project module structure"
        Body  = @"
## Goal
Define modular structure for contributors.

## Tasks
- Create modules: `cli`, `agents`, `core`, `llm`, `fs`, `logger`, `config`

## Suggested PR
`chore: scaffold project modules`
"@
        Labels = @("phase:0", "type:task")
    },
    @{
        Title = "Add tokio async runtime setup"
        Body  = @"
## Goal
Enable async execution across the system.

## Tasks
- Add `tokio`
- Setup async main

## Suggested PR
`chore(runtime): add tokio async runtime`
"@
        Labels = @("phase:0", "type:task")
    },
    @{
        Title = "Implement local-first LLM client"
        Body  = @"
## Goal
Connect to LM Studio-compatible endpoint.

## Tasks
- Add HTTP client with `reqwest`
- Implement POST `/v1/chat/completions`
- Make `base_url` and `model` configurable
- Provide async `generate(prompt: &str) -> Result<String>`

## Suggested PR
`feat(llm): implement local-first LLM client`
"@
        Labels = @("phase:1", "type:feature")
    },
    @{
        Title = "Implement configuration loader"
        Body  = @"
## Goal
Load runtime configuration from `prometheos.config.json`.

## Required fields
- `provider`
- `base_url`
- `model`

## Suggested PR
`feat(config): add config loader`
"@
        Labels = @("phase:1", "type:feature")
    },
    @{
        Title = "Define shared Agent trait interface"
        Body  = @"
## Goal
Create a standard interface for all agents.

## Interface
```rust
pub trait Agent {
    fn name(&self) -> &str;
    async fn run(&self, input: &str) -> Result<String>;
}
```

## Suggested PR
`feat(agents): define agent trait`
"@
        Labels = @("phase:2", "type:feature")
    },
    @{
        Title = "Implement Planner agent"
        Body  = @"
## Goal
Structure tasks into logical execution steps.

## Tasks
- Break input into logical steps
- Return structured plan text

## Suggested PR
`feat(agents): implement planner agent`
"@
        Labels = @("phase:2", "type:feature")
    },
    @{
        Title = "Implement Coder agent"
        Body  = @"
## Goal
Generate files and code from task prompts.

## Tasks
- Call LLM with task prompt
- Return structured output (files + content)

## Suggested PR
`feat(agents): implement coder agent`
"@
        Labels = @("phase:2", "type:feature")
    },
    @{
        Title = "Implement Reviewer agent"
        Body  = @"
## Goal
Improve generated output quality.

## Tasks
- Analyze coder output
- Refine structure and correctness

## Suggested PR
`feat(agents): implement reviewer agent`
"@
        Labels = @("phase:2", "type:feature")
    },
    @{
        Title = "Build sequential orchestrator"
        Body  = @"
## Goal
Coordinate end-to-end agent execution.

## Flow
Planner -> Coder -> Reviewer

## Tasks
- Pass outputs between agents
- Maintain execution context

## Suggested PR
`feat(core): add sequential orchestrator`
"@
        Labels = @("phase:3", "type:feature")
    },
    @{
        Title = "Implement structured agent logger"
        Body  = @"
## Goal
Provide clear agent-based logs.

## Format
- `[Planner] -> ...`
- `[Coder] -> ...`
- `[Reviewer] -> ...`

## Tasks
- Create logger module
- Support streaming logs

## Suggested PR
`feat(logger): implement structured agent logger`
"@
        Labels = @("phase:4", "type:feature")
    },
    @{
        Title = "Add streaming output renderer"
        Body  = @"
## Goal
Display output progressively as it arrives.

## Tasks
- Render text by chunks
- Handle chunked responses reliably

## Suggested PR
`feat(logger): add streaming output renderer`
"@
        Labels = @("phase:4", "type:feature")
    },
    @{
        Title = "Add execution timeline events"
        Body  = @"
## Goal
Improve readability of multi-agent flow.

## Tasks
- Show step transitions
- Emit clear lifecycle events

## Suggested PR
`feat(core): add execution timeline events`
"@
        Labels = @("phase:4", "type:task")
    },
    @{
        Title = "Implement file parser for generated output"
        Body  = @"
## Goal
Extract files from LLM output.

## Tasks
- Detect file blocks
- Extract filenames and content

## Suggested PR
`feat(fs): implement file parser`
"@
        Labels = @("phase:5", "type:feature")
    },
    @{
        Title = "Implement safe file writer"
        Body  = @"
## Goal
Persist generated files safely.

## Tasks
- Create `/prometheos-output`
- Write files safely
- Handle basic conflicts

## Suggested PR
`feat(fs): implement file writer`
"@
        Labels = @("phase:5", "type:feature")
    },
    @{
        Title = "Improve CLI UX output"
        Body  = @"
## Goal
Improve command-line usability.

## Tasks
- Add loading states
- Print output directory
- Add clear success/failure messages

## Suggested PR
`feat(cli): improve CLI UX`
"@
        Labels = @("phase:6", "type:feature")
    },
    @{
        Title = "Add robust error handling and retries"
        Body  = @"
## Goal
Ensure stability under model/API failures.

## Tasks
- Handle LLM failures
- Retry basic requests
- Emit graceful error messages

## Suggested PR
`fix: add error handling and retry logic`
"@
        Labels = @("phase:6", "type:bug")
    },
    @{
        Title = "Optimize default prompts for demo quality"
        Body  = @"
## Goal
Ensure consistent, high-quality outputs in demos.

## Tasks
- Tune prompts
- Validate common use cases

## Suggested PR
`perf: optimize default prompts`
"@
        Labels = @("phase:7", "type:task")
    },
    @{
        Title = "Finalize documentation and examples"
        Body  = @"
## Goal
Complete release-ready docs.

## Tasks
- Final README
- Example commands
- Output samples

## Suggested PR
`docs: finalize documentation`
"@
        Labels = @("phase:7", "type:docs")
    },
    @{
        Title = "Design plugin interface for custom agents (optional)"
        Body  = @"
## Goal
Allow custom agents via a plugin interface.

## Notes
Optional post-launch item.
"@
        Labels = @("phase:optional", "type:feature")
    },
    @{
        Title = "Build basic web log viewer (optional)"
        Body  = @"
## Goal
Provide optional UI for viewing logs.

## Notes
Optional post-launch item.
"@
        Labels = @("phase:optional", "type:feature")
    }
)

$project = $null
try {
    $projectData = Invoke-GithubGraphQL -Query @"
query(`$owner: String!, `$number: Int!) {
  organization(login: `$owner) {
    projectV2(number: `$number) {
      id
      title
    }
  }
}
"@ -Variables @{
        owner  = $Owner
        number = $ProjectNumber
    }

    $project = $projectData.organization.projectV2
}
catch {
    Write-Warning "Could not query ProjectV2. Continuing with issue creation only. Details: $($_.Exception.Message)"
}

if ($project) {
    Write-Host "Project found: $($project.title)"
}
else {
    Write-Warning "Project $Owner/$ProjectNumber not found or not accessible with this token. Issues will still be created."
}

$existingIssuesResponse = Invoke-GithubRest -Method Get -Uri "https://api.github.com/repos/$Owner/$Repo/issues?state=all&per_page=100" -Body $null
$existingIssues = @()
if ($existingIssuesResponse -is [array]) {
    $existingIssues = $existingIssuesResponse
}
elseif ($null -ne $existingIssuesResponse) {
    $existingIssues = @($existingIssuesResponse)
}

foreach ($issue in $issues) {
    $matching = @($existingIssues | Where-Object { $_.title -eq $issue.Title })
    $existing = $null
    if ($matching.Count -gt 0) {
        $existing = $matching[0]
        if ($matching.Count -gt 1) {
            Write-Warning ("Multiple issues matched title '{0}'. Using #{1}." -f $issue.Title, $existing.number)
        }
    }

    if ($existing) {
        $created = $existing
        Write-Host ("Issue already exists: #{0} {1}" -f $created.number, $created.title)
    }
    else {
        $created = Invoke-GithubRest -Method Post -Uri "https://api.github.com/repos/$Owner/$Repo/issues" -Body @{
            title  = $issue.Title
            body   = $issue.Body.Trim()
            labels = $issue.Labels
        }
        $existingIssues += $created
    }

    $contentId = Invoke-GithubGraphQL -Query @"
query(`$url: URI!) {
  resource(url: `$url) {
    ... on Issue {
      id
      url
      number
    }
  }
}
"@ -Variables @{
        url = $created.html_url
    }

    $issueNode = $contentId.resource
    if (-not $issueNode) {
        throw "Could not resolve issue node id for $($created.html_url)"
    }

    if ($project) {
        try {
            Invoke-GithubGraphQL -Query @"
mutation(`$projectId: ID!, `$contentId: ID!) {
  addProjectV2ItemById(input: {
    projectId: `$projectId
    contentId: `$contentId
  }) {
    item {
      id
    }
  }
}
"@ -Variables @{
                projectId = $project.id
                contentId = $issueNode.id
            } | Out-Null

            Write-Host ("Created/verified and added to project: #{0} {1}" -f $created.number, $created.title)
        }
        catch {
            Write-Warning ("Could not add issue #{0} to project: {1}" -f $created.number, $_.Exception.Message)
        }
    }
    else {
        Write-Host ("Created/verified issue only: #{0} {1}" -f $created.number, $created.title)
    }
}

Write-Host "Completed issue sync."
