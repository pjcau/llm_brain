// @ts-check
/** @type {import('@docusaurus/plugin-content-docs').SidebarsConfig} */
const sidebars = {
  docs: [
    'index',
    'changelog',
    {
      type: 'category', label: 'Analysis', collapsed: false,
      items: ['analysis/costs-90-10', 'analysis/hosting-costs', 'analysis/stack', 'analysis/agent-orchestrator', 'analysis/litellm', 'analysis/cli', 'analysis/tool-landscape'],
    },
    {
      type: 'category', label: 'Architecture', collapsed: false,
      items: ['architecture/budget', 'architecture/secrets', 'architecture/auth-topology', 'architecture/auth-flow', 'architecture/client-compatibility', 'architecture/benchmark', 'architecture/api-layer', 'architecture/cache', 'architecture/cache-logic', 'architecture/claude-code-aider', 'architecture/apps', 'architecture/gpu'],
    },
    {
      type: 'category', label: 'Models', collapsed: false,
      items: ['models/bonsai-2-27b'],
    },
    'diagrams',
    'roadmap',
    'decisions',
  ],
};
export default sidebars;
