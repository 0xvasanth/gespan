export type InboxAskContext = {
  key: string;
  title: string;
  url: string;
  provider: "github" | "linear" | "jira" | "gitlab" | "azuredevops";
  description?: string;
};
