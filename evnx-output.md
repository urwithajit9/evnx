```bash
Last login: Sun Feb 15 15:34:39 on ttys001
(base) kumarajit@Kumars-Mac-mini ~ % curl -fsSL https://dotenv.space/install.sh | bash
[INFO] Installing evnx...
[INFO] Detected macOS
[INFO] Target: aarch64-apple-darwin
[INFO] Fetching latest release...
[INFO] Latest version: v0.1.0
[INFO] Downloading from https://github.com/urwithajit9/evnx/releases/download/v0.1.0/evnx-aarch64-apple-darwin.tar.gz
[INFO] Verifying checksum...
evnx-aarch64-apple-darwin.tar.gz: OK
[INFO] Checksum verified ✓
[INFO] Extracting...
[INFO] Installing to /Users/kumarajit/.local/bin...

[INFO] ✓ Installation successful!
[INFO] Installed version: 0.1.0

Quick start:
  evnx init          # Create .env.example
  evnx validate      # Check for issues
  evnx scan          # Detect secrets
  evnx --help        # See all commands

(base) kumarajit@Kumars-Mac-mini ~ % npm create vite@latest evnx-demo -- --template vanilla
Need to install the following packages:
create-vite@8.3.0
Ok to proceed? (y) y


> npx
> create-vite evnx-demo --template vanilla

│
◇  Use Vite 8 beta (Experimental)?:
│  No
│
◇  Install with npm and start now?
│  Yes
│
◇  Scaffolding project in /Users/kumarajit/evnx-demo...
│
◇  Installing dependencies with npm...

added 13 packages, and audited 14 packages in 3s

5 packages are looking for funding
  run `npm fund` for details

found 0 vulnerabilities
│
◇  Starting dev server...

> evnx-demo@0.0.0 dev
> vite


  VITE v7.3.1  ready in 186 ms

  ➜  Local:   http://localhost:5173/
  ➜  Network: use --host to expose
  ➜  press h + enter to show help


```