import antigravity from "../assets/agents/antigravity.svg";
import claude from "../assets/agents/claude.svg";
import codex from "../assets/agents/codex.svg";
import cursor from "../assets/agents/cursor.svg";
import devin from "../assets/agents/devin.svg";
import fx from "../assets/agents/fx.svg";
import grok from "../assets/agents/grok.svg";
import hermes from "../assets/agents/hermes.svg";
import kiro from "../assets/agents/kiro.svg";
import omp from "../assets/agents/omp.svg";
import opencode from "../assets/agents/opencode.svg";
import pi from "../assets/agents/pi.svg";

/** The agents README.md's Supported agents table lists, in its order. */
export const AGENTS = [
  { name: "Claude Code", logo: claude },
  { name: "Codex", logo: codex },
  { name: "Cursor CLI", logo: cursor },
  { name: "Grok Build", logo: grok },
  { name: "OpenCode", logo: opencode },
  { name: "Antigravity", logo: antigravity },
  { name: "Pi", logo: pi },
  { name: "omp", logo: omp },
  { name: "fx", logo: fx },
  { name: "Hermes Agent", logo: hermes },
  { name: "Kiro CLI", logo: kiro },
  { name: "Devin CLI", logo: devin },
];

export function AgentWall() {
  return (
    <ul className="w-g6 w-glass-flat" style={{ padding: "8px", gap: "4px", listStyle: "none", margin: 0 }} aria-label="Supported agents">
      {AGENTS.map((agent) => (
        <li key={agent.name} className="w-agent">
          <img src={agent.logo} alt="" width="40" height="40" />
          <span>{agent.name}</span>
        </li>
      ))}
    </ul>
  );
}
