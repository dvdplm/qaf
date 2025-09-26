# GitHub Actions Workflows

This directory contains GitHub Actions workflows for building and releasing qaf.

## Workflows

### Build (`build.yml`)
**Triggers:** Push to main/master, Pull Requests, Manual dispatch

Builds qaf for Apple Silicon (M1/M2/M3 Macs) and runs tests.

**Artifacts produced:**
- `qaf-apple-silicon` - Apple Silicon Mac binary

### Release (`release.yml`)
**Triggers:** Git tags matching `v*.*.*` or `v*.*`, Manual dispatch

Creates production releases with:
- Optimized, stripped binary for Apple Silicon
- Compressed archive (.tar.gz)
- SHA256 checksum
- GitHub Release with installation instructions

## Creating a Release

1. **Update version in Cargo.toml:**
   ```toml
   [package]
   version = "1.0.0"
   ```

2. **Commit and tag:**
   ```bash
   git add Cargo.toml
   git commit -m "chore: bump version to 1.0.0"
   git tag v1.0.0
   git push origin main --tags
   ```

3. The Release workflow will automatically:
   - Build optimized binary
   - Create GitHub Release
   - Upload artifacts with checksum

## Manual Workflow Triggers

You can manually trigger workflows from the GitHub Actions tab:

1. Go to Actions tab
2. Select the workflow
3. Click "Run workflow"
4. Choose branch and fill in any inputs