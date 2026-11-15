import { LazyMotion, MotionConfig, domAnimation } from "motion/react";
import { useEffect, type ComponentType } from "react";
import { ChangelogPage } from "./pages/ChangelogPage";
import { DocsPage } from "./pages/DocsPage";
import { DownloadPage } from "./pages/DownloadPage";
import { FeaturesPage } from "./pages/FeaturesPage";
import { HomePage } from "./pages/HomePage";
import { NotFoundPage } from "./pages/NotFoundPage";
import { PilotsPage } from "./pages/PilotsPage";
import { useReveal } from "./reveal";
import { matchRoute, usePathname, type Page } from "./router";

const PAGES: Record<Page, ComponentType> = {
  home: HomePage,
  features: FeaturesPage,
  pilots: PilotsPage,
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
  const Page = route ? PAGES[route.page] : NotFoundPage;
  return (
    <LazyMotion features={domAnimation} strict>
      <MotionConfig reducedMotion="user">
        <main>
          <Page />
        </main>
      </MotionConfig>
    </LazyMotion>
  );
}
