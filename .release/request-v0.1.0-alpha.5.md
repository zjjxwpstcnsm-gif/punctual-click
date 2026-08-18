# Release request

Build and publish Punctual v0.1.0-alpha.5 from the current `main` branch.

The owner-only `pull_request_target` workflow checks the PR author and exact title, checks out only `main`, recompiles macOS ARM64 and Windows x64 packages, verifies checksums, and publishes a prerelease.
