# Bundled armature resource

The SwiftPM executable target can optionally bundle an armature resource here:

- `BearsApp/Resources/Adapter/bear-armature`
- `BearsApp/Resources/Adapter/bears-acp-adapter` (legacy fallback)

Populate it with:

```bash
cd apps/apple/Bears
bash Scripts/prepare_adapter.sh
```

If this file is absent, the app now falls back to downloading a macOS armature artifact from GitHub using its configured download URL.
