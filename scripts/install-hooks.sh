#!/bin/sh
cp scripts/pre-push .git/hooks/pre-push && chmod +x .git/hooks/pre-push && echo "pre-push hook installed"
