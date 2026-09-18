# Template-specific verification guidance

For `agent-instructions.md`, after the `tandem-agents.md` paragraph:

Make verification instructions executable: for each application, specify the test command, service, container working directory, and required setup. Identify available verification tools and whether they run on the host or in a container. If no automated suite exists, say so and provide a concrete smoke-check procedure with expected results. Verify commands against the environment; do not assume tooling is installed. Keep guidance template-specific and instance-independent, using the generated workspace service mappings and URLs rather than hardcoded instance names or ports.
