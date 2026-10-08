# Security policy

Playground is pre-alpha. If you find a problem that could expose a user's data (for example an API key or sign-in token reaching a log, save, export or crash report, or a way for a content pack to escape the script sandbox), please report it privately through GitHub's "Report a vulnerability" button on the Security tab of this repository rather than opening a public issue. Include the steps to reproduce and the version or commit.

What is in scope: the simulation core, the scripting sandbox, save and export handling, and the AI provider and sign-in code. Keys must never reach logs, saves, exports or crash reports (Blueprint section 17); a report that shows otherwise is treated as a serious bug.
