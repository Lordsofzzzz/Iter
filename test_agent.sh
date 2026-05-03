#!/bin/bash
export OPENROUTER_API_KEY=""
cd agent
bun run src/index.ts << 'JSON_EOF'
{"type":"prompt","content":"Hello!"}
JSON_EOF
