import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { AgentTranscript } from "../../src/features/sessions/ui/AgentTranscript";
import { PilotComposer } from "../../src/features/pilots/ui/PilotComposer";
import { MessageQueue } from "../../src/features/sessions/ui/MessageQueue";
import { visiblePilotCompletionMessages } from "../../src/features/pilots/model/pilotMessaging";
import {
  newSession,
  type Block,
  type QueuedMessage,
} from "../../src/features/sessions/model/session";
import {
  saveTranscriptAnchor,
  saveTranscriptLayout,
} from "../../src/features/settings/model/appearance";
import "../../src/styles/index.css";

saveTranscriptAnchor(false);
saveTranscriptLayout("chat");

// Settled plain text isolates layout during scrolling from streaming, image
// loading and asynchronous syntax highlighting. Turns deliberately vary in size.
const blocks: Block[] = Array.from({ length: 20 }, (_, index): Block[] => [
  {
    id: `user-${index}`,
    role: "user",
    text: `Prompt ${index}: explain this behavior and preserve my place while reading.`,
  },
  {
    id: `reply-${index}`,
    role: "assistant",
    text: Array.from(
      { length: 8 + (index % 5) * 8 },
      (_, paragraph) =>
        `Paragraph ${paragraph}. This is a settled answer with enough text to exercise variable message heights.`,
    ).join("\n\n"),
  },
]).flat();

// Like the transcript pool, parking detaches the transcript from the page.
const root = document.getElementById("root")!;
const host = document.createElement("div");
host.className = "contents";
root.appendChild(host);

function Fixture() {
  const [parked, setParked] = useState(false);
  useEffect(() => {
    Object.assign(window, {
      parkTranscript: () => {
        host.remove();
        setParked(true);
      },
      showTranscript: () => {
        root.appendChild(host);
        setParked(false);
      },
    });
  }, []);
  return (
    <AgentTranscript
      blocks={blocks}
      initialTurns={20}
      visible={!parked}
      parked={parked}
    />
  );
}

const pilotAnswer =
  "A reply arriving in chunks that should glide smoothly as the conversation grows. ".repeat(
    35,
  );

function PilotFixture() {
  const history = blocks.slice(-12);
  const prompt: Block = {
    id: "pilot-prompt",
    role: "user",
    text: "Explain this in detail",
  };
  const [busy, setBusy] = useState(true);
  const [conversation, setConversation] = useState<Block[]>([
    ...history,
    prompt,
    { id: "pilot-answer", role: "assistant", text: pilotAnswer, streaming: true },
  ]);
  useEffect(() => {
    Object.assign(window, {
      completePilotReply: (text = pilotAnswer) => {
        setConversation([
          ...history,
          prompt,
          { id: "pilot-answer", role: "assistant", text, streaming: false },
        ]);
        setBusy(false);
      },
    });
  }, []);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="min-h-0 flex-1">
        <AgentTranscript
          blocks={conversation}
          busy={busy}
          agentName="Pilot"
          agentMascot={{ mascot: "crab", color: "#aaf" }}
          bottomAligned
          inlineWork
        />
      </div>
      <PilotComposer
        sessionId="browser-pilot"
        name="Pilot"
        enabled={false}
        onSubmit={() => false}
        controls={<span className="h-6.5">Model · Effort · Context</span>}
      />
    </div>
  );
}

function PilotCompletionFixture() {
  const history = new URLSearchParams(location.search).has("short")
    ? []
    : blocks.slice(-12);
  const start = Date.now();
  const launched = {
    sessionId: "app-pilot-review",
    cwd: "/repo",
    title: "Review the changes",
    harness: "codex" as const,
    model: "gpt-6",
  };
  const original: Block[] = [
    ...history,
    {
      id: "pilot-request",
      role: "user",
      text: "Start a review session and report back",
      startedAt: start,
      durationMs: 1000,
      pilotSpawnedSessions: [launched],
    },
    {
      id: "pilot-launch",
      role: "tool",
      text: "Start a session",
      tool: { kind: "execute", status: "completed" },
    },
    {
      id: "pilot-handoff",
      role: "assistant",
      text: "I started the review session. I’ll report back when it finishes.",
    },
  ];
  const completion: Block = {
    id: "pilot-completion",
    role: "user",
    text: "Review complete",
    internal: true,
    appRequestId: "pilot-completion-batch-review",
    startedAt: start + 5000,
  };
  const [conversation, setConversation] = useState(original);
  const [notifications, setNotifications] = useState<QueuedMessage[]>([]);
  const [session] = useState(() => newSession("codex", "/repo"));
  const [busy, setBusy] = useState(false);
  const [state, setState] = useState<"working" | "done">("working");
  useEffect(() => {
    Object.assign(window, {
      queuePilotCompletion: () => {
        setNotifications([
          {
            id: completion.id,
            text: completion.text,
            attachments: [],
            pilotSessionCompletion: {
              sessionId: launched.sessionId,
              title: launched.title,
              status: "completed",
            },
          },
        ]);
        setState("done");
      },
      beginPilotCompletion: () => {
        setNotifications([]);
        setConversation([...original, completion]);
        setBusy(true);
        setState("done");
      },
      finishPilotCompletion: () => {
        setConversation([
          ...original,
          { ...completion, durationMs: 1000 },
          {
            id: "pilot-report",
            role: "assistant",
            text: "The review is complete. Here are the results. ".repeat(40),
          },
        ]);
        setBusy(false);
      },
    });
  }, []);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="min-h-0 flex-1">
        <AgentTranscript
          blocks={conversation}
          busy={busy}
          agentName="Pilot"
          agentMascot={{ mascot: "crab", color: "#aaf" }}
          bottomAligned
          inlineWork
          onOpenSession={() => {}}
          onShowWork={() => {}}
          launchedSessions={
            new Map([[launched.sessionId, { state, title: launched.title }]])
          }
        />
      </div>
      <MessageQueue
        messages={visiblePilotCompletionMessages({
          ...session,
          blocks: conversation,
          busy,
          queuedMessages: notifications,
          queueStatus: "active",
        })}
        variant="messages"
        onResume={() => {}}
      />
      <PilotComposer
        sessionId="completion-pilot"
        name="Pilot"
        enabled={false}
        onSubmit={() => false}
        controls={<span className="h-6.5">Model · Effort · Context</span>}
      />
    </div>
  );
}

const params = new URLSearchParams(location.search);
const pilot = params.has("pilot");
const completion = params.has("pilotCompletion");
if (pilot || completion) root.style.width = "540px";
createRoot(host).render(
  completion ? <PilotCompletionFixture /> : pilot ? <PilotFixture /> : <Fixture />,
);
