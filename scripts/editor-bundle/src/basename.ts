export const basename = (path: string) =>
  path.replace(/\\/g, "/").replace(/\/+$/, "").split("/").pop() || "/";
