#!/bin/bash
export OPENROUTER_API_KEY="sk-or-v1-YOUR_KEY_HERE"
export MODEL_NAME="minimax/minimax-m2.5:free"
cd agent
bun run src/index.ts << 'JSON_EOF'
{"type":"prompt","content":"Hello"}
JSON_EOF