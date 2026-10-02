# Security Policy

## Supported version

Security fixes are applied to the current development version of PIC. Users should reproduce security issues with the latest available release or current `main` branch when practical.

## Data safety

PIC is designed as a non-destructive catalogue: its database references photos stored in their existing locations and normal catalogue operations should not delete original photo files. The database is not a backup of those originals.

Because PIC is a personal passion project under active development, test with temporary/sample data first and maintain independent backups of important photos and catalogue data. Bugs, filesystem problems, storage failures and network-share failures can still cause data loss outside the intended application behaviour.

Any behaviour that unexpectedly modifies, overwrites or deletes an original file should be treated as a serious bug and reported promptly.

## Reporting a vulnerability

Please do **not** open a public GitHub issue for a vulnerability that could put users, their files, credentials, network shares, or systems at risk.

Report the issue privately to the project maintainer using the contact information on the maintainer's GitHub profile. Include a clear description, affected version or commit, reproduction steps, potential impact, and any suggested mitigation if known.

Please allow reasonable time for investigation and a fix before publicly disclosing a vulnerability.

Ordinary bugs that do not have a security impact can be reported through GitHub Issues.
