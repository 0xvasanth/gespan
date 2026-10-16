import { useLayoutEffect, type RefObject } from "react";

type Anchor = {
  message: HTMLElement;
  node: Text;
  offset: number;
  character: number;
  top: number;
};

/** Keep the reader's character in view through file reflow and reply completion. */
export function useReplyReflowAnchor(
  scroller: HTMLDivElement | null,
  visible: boolean,
  following: RefObject<boolean>,
  remember: (element: HTMLElement) => void,
) {
  useLayoutEffect(() => {
    if (!visible || !scroller) return;
    let width = scroller.clientWidth;
    let anchor: Anchor | undefined;
    let frame = 0;
    let adjustedTop: number | undefined;
    let manualUntil = 0;
    let draggingScroll = false;
    const capture = () => {
      if (following.current) {
        anchor = undefined;
        return;
      }
      // A pending width change must use the position from before reflow.
      if (!scroller.isConnected || scroller.clientWidth !== width) return;
      const viewport = scroller.getBoundingClientRect();
      for (const fraction of [0.25, 0.5, 0.75]) {
        const range = document.caretRangeFromPoint?.(
          viewport.left + viewport.width * fraction,
          viewport.top + 32,
        );
        if (!range || range.startContainer.nodeType !== Node.TEXT_NODE)
          continue;
        const node = range.startContainer as Text;
        const message = node.parentElement?.closest<HTMLElement>(
          '[data-chat-message-role="assistant"]',
        );
        if (!message || !scroller.contains(message) || !node.length) continue;
        const offset = Math.min(range.startOffset, node.length - 1);
        range.setStart(node, offset);
        range.setEnd(node, offset + 1);
        const prefix = document.createRange();
        prefix.selectNodeContents(message);
        prefix.setEnd(node, offset);
        anchor = {
          message,
          node,
          offset,
          character: prefix.toString().length,
          top: range.getBoundingClientRect().top - viewport.top,
        };
        return;
      }
    };
    const schedule = () => {
      if (!frame)
        frame = requestAnimationFrame(() => {
          frame = 0;
          capture();
        });
    };
    const onScroll = () => {
      if (
        adjustedTop != null &&
        Math.abs(scroller.scrollTop - adjustedTop) < 1
      ) {
        adjustedTop = undefined;
        return;
      }
      adjustedTop = undefined;
      // Other transcript observers also correct scrollTop during reflow.
      // Those corrections must not replace the reader's original character.
      if (!anchor || draggingScroll || performance.now() < manualUntil)
        schedule();
    };
    const manualScroll = () => {
      manualUntil = performance.now() + 180;
      schedule();
    };
    const pointerDown = () => {
      draggingScroll = true;
    };
    const pointerUp = () => {
      draggingScroll = false;
    };
    const keyDown = (event: KeyboardEvent) => {
      if (
        [
          "ArrowUp",
          "ArrowDown",
          "PageUp",
          "PageDown",
          "Home",
          "End",
          " ",
        ].includes(event.key)
      )
        manualScroll();
    };
    const resize = new ResizeObserver(() => {
      const next = scroller.clientWidth;
      if (!next || !scroller.isConnected) return;
      const reflow = next !== width;
      if (reflow && anchor) {
        // A wheel's pending capture must not choose a different character
        // after the divider has already changed the line wrapping.
        manualUntil = 0;
        if (frame) cancelAnimationFrame(frame);
        frame = 0;
      } else if (draggingScroll || performance.now() < manualUntil) {
        capture();
        width = next;
        return;
      }
      if (!following.current && anchor?.message.isConnected) {
        let { node, offset } = anchor;
        if (!node.isConnected || offset >= node.length) {
          // Markdown can replace a text node while retaining the same text.
          let remaining = anchor.character;
          const walker = document.createTreeWalker(
            anchor.message,
            NodeFilter.SHOW_TEXT,
          );
          let found: Text | null;
          while ((found = walker.nextNode() as Text | null)) {
            if (remaining < found.length) {
              node = found;
              offset = remaining;
              break;
            }
            remaining -= found.length;
          }
        }
        if (node.isConnected && offset < node.length) {
          const range = document.createRange();
          range.setStart(node, offset);
          range.setEnd(node, offset + 1);
          const shift =
            range.getBoundingClientRect().top -
            scroller.getBoundingClientRect().top -
            anchor.top;
          if (Math.abs(shift) > 0.5) {
            scroller.scrollTop += shift;
            adjustedTop = scroller.scrollTop;
            remember(scroller);
          }
        }
      }
      width = next;
      // Keep this character for the whole drag. Picking another word at the
      // same x coordinate after each reflow would gradually drift by lines.
      if (!anchor) capture();
    });
    resize.observe(scroller);
    // Completion can remove a status row inside the reply's own turn. The
    // whole-turn observer only corrects turns above the reader, so retain the
    // character here even when the viewport width itself has not changed.
    if (scroller.firstElementChild) resize.observe(scroller.firstElementChild);
    scroller.addEventListener("scroll", onScroll, { passive: true });
    scroller.addEventListener("wheel", manualScroll, { passive: true });
    scroller.addEventListener("touchmove", manualScroll, { passive: true });
    scroller.addEventListener("pointerdown", pointerDown, { passive: true });
    scroller.addEventListener("keydown", keyDown);
    document.addEventListener("pointerup", pointerUp, { passive: true });
    schedule();
    return () => {
      resize.disconnect();
      scroller.removeEventListener("scroll", onScroll);
      scroller.removeEventListener("wheel", manualScroll);
      scroller.removeEventListener("touchmove", manualScroll);
      scroller.removeEventListener("pointerdown", pointerDown);
      scroller.removeEventListener("keydown", keyDown);
      document.removeEventListener("pointerup", pointerUp);
      if (frame) cancelAnimationFrame(frame);
    };
  }, [scroller, visible, following, remember]);
}
