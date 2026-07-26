"use strict";

const fs = require("node:fs");

const BLOCK_PATTERNS = [
  /\/\*\s*agent-research:start\s*\*\/[\s\S]*?\/\*\s*agent-research:end\s*\*\//g,
  /<!--\s*agent-research:start\s*-->[\s\S]*?<!--\s*agent-research:end\s*-->/g,
];

const FORBIDDEN_PRODUCTION_PATTERNS = [
  ["/api/agent/", /\/api\/agent\//],
  ["Agent view", /id=["']v-agents["']/],
  ["Agent shortcut command", /set_agent_shortcut/],
  ["Agent compute settings", /agentCompute/],
  ["Agent WebView entrypoint", /__myAssistantOpenAgents/],
  ["Agent shortcut setting", /agent_shortcut/],
  ["Agent experiment setting", /agents_exp/],
];

function stripAgentResearch(source) {
  let stripped = source;
  let removedBlocks = 0;
  for (const pattern of BLOCK_PATTERNS) {
    stripped = stripped.replace(pattern, () => {
      removedBlocks += 1;
      return "";
    });
  }
  if (removedBlocks === 0) {
    throw new Error("No Agent research markers were found in the release HTML source");
  }
  if (/agent-research:(?:start|end)/.test(stripped)) {
    throw new Error("Unbalanced Agent research markers remain in the release HTML");
  }
  for (const [label, pattern] of FORBIDDEN_PRODUCTION_PATTERNS) {
    if (pattern.test(stripped)) {
      throw new Error(`${label} leaked into the production HTML`);
    }
  }
  if (!/data-view=["']agents["'][^>]*disabled[^>]*aria-disabled=["']true["']/.test(stripped)) {
    throw new Error("The disabled Agents Coming soon navigation label must remain visible");
  }
  return stripped;
}

if (require.main === module) {
  const [input, output] = process.argv.slice(2);
  if (!input || !output) {
    throw new Error("usage: prepare_release_html.cjs INPUT OUTPUT");
  }
  const source = fs.readFileSync(input, "utf8");
  fs.writeFileSync(output, stripAgentResearch(source));
}

module.exports = {
  FORBIDDEN_PRODUCTION_PATTERNS,
  stripAgentResearch,
};
