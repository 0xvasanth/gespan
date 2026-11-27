import { motion, useInView, useReducedMotion } from "motion/react";
import { useRef, useState } from "react";
import "./terminal.css";

export type TerminalLine = { kind: "comment"; text: string } | { kind: "cmd"; text: string; alt?: string };

/**
 * The quick start's terminal. Every command here is one the docs give
 * (docs/providers.md, docs/install.md); comments say what the step is for.
 */
export const QUICK_START: TerminalLine[] = [
  { kind: "comment", text: "1. Sign in to at least one agent. Gespan reuses its login." },
  { kind: "cmd", text: "claude auth login", alt: "or codex login, agent login, opencode auth login" },
  { kind: "comment", text: "2. Install Gespan. On macOS and Windows, use the download page." },
  { kind: "cmd", text: "sudo apt install ./Gespan_*.deb", alt: "Ubuntu and Debian" },
  { kind: "cmd", text: "sudo dnf install ./Gespan-*.rpm", alt: "Fedora" },
  { kind: "comment", text: "3. Open Gespan and carry on in the app, below." },
];

/** How long after the terminal comes into view line `i` appears (s); all at once for reduced motion. */
export function lineDelay(i: number, reduced: boolean): number {
  return reduced ? 0 : 0.15 + i * 0.32;
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    navigator.clipboard?.writeText(text).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1600);
      },
      () => {},
    );
  };
  return (
    <button type="button" className="tm-copy" onClick={copy} aria-label={copied ? `Copied ${text}` : `Copy ${text}`}>
      {copied ? "Copied" : "Copy"}
    </button>
  );
}

/** A terminal window whose lines appear one by one as it scrolls into view. */
export function Terminal({ title, lines }: { title: string; lines: TerminalLine[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const seen = useInView(ref, { once: true, amount: 0.4 });
  const reduced = !!useReducedMotion();
  const shown = seen || reduced;
  return (
    <div ref={ref} className="tm" data-terminal="">
      <div className="tm-bar">
        <span className="tm-dots" aria-hidden="true"><i /><i /><i /></span>
        <span className="tm-title">{title}</span>
      </div>
      <ol className="tm-body">
        {lines.map((line, i) => (
          <motion.li
            key={line.text}
            className={`tm-line ${line.kind}`}
            initial={reduced ? false : { opacity: 0, y: 4 }}
            animate={shown ? { opacity: 1, y: 0 } : undefined}
            transition={{ duration: 0.3, delay: lineDelay(i, reduced) }}
          >
            {line.kind === "comment" ? (
              <span># {line.text}</span>
            ) : (
              <>
                <span className="tm-prompt" aria-hidden="true">$</span>
                <code>{line.text}</code>
                {line.alt && <span className="tm-alt"># {line.alt}</span>}
                <CopyButton text={line.text} />
              </>
            )}
          </motion.li>
        ))}
        <li className="tm-line cursor" aria-hidden="true"><span className="tm-prompt">$</span><span className={reduced ? "tm-caret still" : "tm-caret"} /></li>
      </ol>
    </div>
  );
}
