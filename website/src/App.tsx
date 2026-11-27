import { MotionConfig } from "motion/react";
import { ScrollProgress } from "@/components/ui/scroll-progress";
import { SiteNav } from "./components/SiteNav";
import { useEffect, type ComponentType } from "react";
import { ChangelogPage } from "./pages/ChangelogPage";
import { DocsPage } from "./pages/DocsPage";
import { DownloadPage } from "./pages/DownloadPage";
import { FeaturesPage } from "./pages/FeaturesPage";
import { HomePage } from "./pages/HomePage";
import { CloudPage } from "./pages/CloudPage";
import { NotFoundPage } from "./pages/NotFoundPage";
import { PilotsPage } from "./pages/PilotsPage";
import { useReveal } from "./reveal";
import { matchRoute, redirectFor, usePathname, type Page } from "./router";

const PAGES: Record<Page, ComponentType> = {
  home: HomePage,
  features: FeaturesPage,
  pilots: PilotsPage,
  cloud: CloudPage,
  download: DownloadPage,
  changelog: ChangelogPage,
  docs: DocsPage,
};

export function App() {
  const pathname = usePathname();
  const route = matchRoute(pathname);
  useReveal(pathname);
  const title = route?.title ?? "Page not found · Gespan";
  useEffect(() => {
    document.title = title;
  }, [title]);
  // An old address shows its new page and swaps itself for the new address.
  const moved = redirectFor(pathname);
  useEffect(() => {
    if (moved) window.history.replaceState(null, "", moved + window.location.hash);
  }, [moved]);
  const Page = route ? PAGES[route.page] : NotFoundPage;
  return (
    <MotionConfig reducedMotion="user">
      <ScrollProgress className="h-0.5 from-[#c0affc] via-[#9f7ef8] to-[#7e47ea]" />
      <SiteNav active={route?.page ?? "home"} />
      <main>
        <Page />
      </main>
    </MotionConfig>
  );
}
