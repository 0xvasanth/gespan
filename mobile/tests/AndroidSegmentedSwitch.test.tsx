import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { SegmentedSwitch } from "@/ui/SegmentedSwitch.android";

vi.mock("expo-haptics", () => ({ selectionAsync: vi.fn() }));
vi.mock("react-native", () => ({
  View: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
  Text: ({ children }: { children?: ReactNode }) => <span>{children}</span>,
  Pressable: ({ children, onPress, accessibilityLabel, accessibilityState }: {
    children?: ReactNode;
    onPress: () => void;
    accessibilityLabel: string;
    accessibilityState: { selected: boolean };
  }) => <button aria-label={accessibilityLabel} aria-selected={accessibilityState.selected} onClick={onPress}>{children}</button>,
}));
vi.mock("@/ui/LiquidView", () => ({
  LiquidView: ({ children, style }: { children: ReactNode; style?: object }) => <div style={style}>{children}</div>,
}));
vi.mock("@/ui/theme", () => ({
  themedStyles: (create: (colors: object) => object) => () => create({ content: "white", thumb: "gray" }),
}));

it("keeps the selected label visible without layout events, across switches and remounts", () => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  const root = createRoot(container);
  const onChange = vi.fn();
  const segments = [
    { value: "new", label: "New", icon: <i data-icon="compose" /> },
    { value: "sessions", label: "Sessions", icon: <i data-icon="list" /> },
  ];
  const render = (value: string, key = "first") => act(() => root.render(
    <SegmentedSwitch key={key} segments={segments} value={value} onChange={onChange} />,
  ));
  try {
    // No onLayout events or animated progress updates are delivered.
    render("new");
    expect(container.textContent).toBe("New");
    expect(container.querySelectorAll("[data-icon]")).toHaveLength(2);
    const buttons = container.querySelectorAll("button");
    act(() => buttons[0].click());
    expect(onChange).not.toHaveBeenCalled();
    act(() => buttons[1].click());
    expect(onChange).toHaveBeenCalledWith("sessions");
    render("sessions");
    expect(container.textContent).toBe("Sessions");
    expect(container.querySelector('[aria-selected="true"]')?.textContent).toBe("Sessions");
    render("new", "remounted");
    expect(container.textContent).toBe("New");
    expect((container.firstElementChild as HTMLElement).style.width).toBe("200px");
    expect((container.firstElementChild as HTMLElement).style.maxWidth).toBe("100%");
  } finally {
    act(() => root.unmount());
  }
});
