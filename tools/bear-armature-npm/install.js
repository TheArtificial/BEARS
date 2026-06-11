#!/usr/bin/env node
// Downloads the platform-appropriate bear-armature binary from GitHub Releases.
// Runs automatically as a postinstall script.
"use strict";

const https = require("https");
const fs = require("fs");
const path = require("path");
const { execSync } = require("child_process");
const os = require("os");

const VERSION = require("./package.json").version;
const BIN_DIR = path.join(__dirname, "bin");
const EXE =
    process.platform === "win32" ? "bear-armature.exe" : "bear-armature";
const LEGACY_EXE =
    process.platform === "win32"
        ? "bears-acp-adapter.exe"
        : "bears-acp-adapter";
const BIN_PATH = path.join(BIN_DIR, EXE);

const PLATFORM_MAP = {
    "darwin:arm64": "aarch64-apple-darwin",
    "darwin:x64": "x86_64-apple-darwin",
    "linux:x64": "x86_64-unknown-linux-musl",
    "win32:x64": "x86_64-pc-windows-msvc",
};

const key = `${process.platform}:${process.arch}`;
const triple = PLATFORM_MAP[key];

if (!triple) {
    console.warn(
        `bear-armature: unsupported platform ${key} — skipping binary download.`,
    );
    console.warn(
        "Build from source: cargo build --release --manifest-path tools/bear-armature/Cargo.toml",
    );
    process.exit(0);
}

const ext = process.platform === "win32" ? ".zip" : ".tar.gz";
const filename = `bear-armature-${triple}${ext}`;
const legacyFilename = `bears-acp-adapter-${triple}${ext}`;
const tag = `bear-armature%2Fv${VERSION}`;
const legacyTag = `bears-acp-adapter%2Fv${VERSION}`;
const url = `https://github.com/bears-ai/bear-den/releases/download/${tag}/${filename}`;
const legacyUrl = `https://github.com/bears-ai/bear-den/releases/download/${legacyTag}/${legacyFilename}`;
const tmp = path.join(os.tmpdir(), filename);

function download(url, dest, redirects = 0) {
    return new Promise((resolve, reject) => {
        if (redirects > 5) return reject(new Error("Too many redirects"));
        https
            .get(url, (res) => {
                if (
                    res.statusCode >= 300 &&
                    res.statusCode < 400 &&
                    res.headers.location
                ) {
                    res.resume();
                    return download(
                        res.headers.location,
                        dest,
                        redirects + 1,
                    ).then(resolve, reject);
                }
                if (res.statusCode !== 200) {
                    res.resume();
                    return reject(
                        new Error(`HTTP ${res.statusCode} fetching ${url}`),
                    );
                }
                const file = fs.createWriteStream(dest);
                res.pipe(file);
                file.on("finish", () => file.close(resolve));
                file.on("error", reject);
            })
            .on("error", reject);
    });
}

async function main() {
    console.log(`bear-armature: downloading ${filename}…`);

    try {
        await download(url, tmp);
    } catch (err) {
        console.warn(`bear-armature: download failed: ${err.message}`);
        console.log(`bear-armature: trying legacy release ${legacyFilename}…`);
        try {
            await download(legacyUrl, tmp);
        } catch (legacyErr) {
            console.warn(
                `bear-armature: legacy download failed: ${legacyErr.message}`,
            );
            console.warn(
                "Build from source: cargo build --release --manifest-path tools/bear-armature/Cargo.toml",
            );
            process.exit(0);
        }
    }

    fs.mkdirSync(BIN_DIR, { recursive: true });

    if (process.platform === "win32") {
        execSync(
            `powershell.exe -NoProfile -Command "Expand-Archive -Force '${tmp}' '${BIN_DIR}'"`,
            { stdio: "inherit" },
        );
    } else {
        execSync(`tar -xzf '${tmp}' -C '${BIN_DIR}'`, { stdio: "inherit" });
    }

    try {
        fs.unlinkSync(tmp);
    } catch (_) {}

    if (process.platform !== "win32") {
        fs.chmodSync(BIN_PATH, 0o755);
        const legacyPath = path.join(BIN_DIR, LEGACY_EXE);
        try {
            if (fs.existsSync(legacyPath)) fs.unlinkSync(legacyPath);
            fs.symlinkSync(EXE, legacyPath);
        } catch (_) {}
    }

    console.log(`bear-armature: installed to ${BIN_PATH}`);
}

main().catch((err) => {
    console.error(`bear-armature: install error: ${err.message}`);
    process.exit(0); // non-fatal so npm install does not fail
});
