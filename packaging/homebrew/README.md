# Homebrew tap for bear-armature

```text
Formula/
    bear-armature.rb
    bears-acp-adapter.rb   # legacy formula alias
```

Install:

```bash
brew install bear-armature
```

Legacy formula name (installs the same binary plus `bears-acp-adapter` symlink):

```bash
brew install bears-acp-adapter
```

The release workflow (`.github/workflows/bear-armature.yml`) prints SHA256 sums for all artifacts. After pushing a `bear-armature/v*` tag:

1. Open the workflow run for that tag.
2. Copy the hashes for `bear-armature-aarch64-apple-darwin.tar.gz` and `bear-armature-x86_64-apple-darwin.tar.gz`.
3. Update the `sha256` fields in `Formula/bear-armature.rb` and bump `version`.

Quarantine (local testing):

```bash
xattr -d com.apple.quarantine /path/to/bear-armature
```
