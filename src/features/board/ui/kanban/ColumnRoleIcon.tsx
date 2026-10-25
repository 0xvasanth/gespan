import type { ColumnRole } from "../../model/board";

const COLOR: Record<ColumnRole | "none", string> = {
  todo: "text-content/45",
  doing: "text-skill",
  review: "text-accent",
  done: "text-diff-add",
  cancelled: "text-content/45",
  none: "text-content/45",
};

/**
 * 14px ring for a column role: todo dashed, doing half filled, review three
 * quarters filled, done filled with a check, cancelled struck through, no
 * role a plain ring.
 */
export function ColumnRoleIcon({
  role,
  className = "",
}: {
  role: ColumnRole | null;
  className?: string;
}) {
  const key = role ?? "none";
  return (
    <svg
      width={14}
      height={14}
      viewBox="0 0 14 14"
      fill="none"
      aria-hidden="true"
      data-role={key}
      className={`shrink-0 ${COLOR[key]} ${className}`}
    >
      {role === "done" ? (
        <>
          <circle cx={7} cy={7} r={6.25} fill="currentColor" />
          <path
            d="M4.4 7.2 6.2 9 9.6 5.4"
            stroke="var(--color-background-base)"
            strokeWidth={1.5}
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </>
      ) : (
        <>
          <circle
            cx={7}
            cy={7}
            r={5.75}
            stroke="currentColor"
            strokeWidth={1.5}
            strokeDasharray={role === "todo" ? "2.2 1.8" : undefined}
          />
          {role === "doing" && (
            <path d="M7 7V3.5a3.5 3.5 0 0 1 0 7Z" fill="currentColor" />
          )}
          {role === "review" && (
            <path d="M7 7V3.5a3.5 3.5 0 1 1-3.5 3.5Z" fill="currentColor" />
          )}
          {role === "cancelled" && (
            <path
              data-mark="strike"
              d="M3 11 11 3"
              stroke="currentColor"
              strokeWidth={1.5}
              strokeLinecap="round"
            />
          )}
        </>
      )}
    </svg>
  );
}
