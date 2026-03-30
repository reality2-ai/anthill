#!/bin/bash
cd "$(dirname "$0")/../.."
python3 web/dashboard/parse_specs.py
python3 web/dashboard/calculate.py
python3 web/dashboard/generate.py
python3 web/dashboard/generate_readme.py
echo "Dashboard generated: web/dashboard/index.html + README.md updated"
