# Harness Patch Protocol

The production patch path accepts structured JSON `EditOperation` values. Supported operations include search/replace, whole file, create file, delete file, rename file, and unified diff validation. Atomic application currently executes structured non-diff operations.
