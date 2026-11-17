import { useEffect, useLayoutEffect, useState, type RefObject } from "react";

/** Tracks the scene element's width and the system's light-appearance preference. */
export function useSceneViewport(scene: RefObject<HTMLElement | null>) {
  const [width, setWidth] = useState(720);
  const [systemLight, setSystemLight] = useState(false);

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: light)");
    const change = () => setSystemLight(media.matches);
    change();
    media.addEventListener("change", change);
    return () => media.removeEventListener("change", change);
  }, []);

  useLayoutEffect(() => {
    if (!scene.current) return;
    setWidth(Math.max(360, scene.current.getBoundingClientRect().width));
    const observer = new ResizeObserver((entries) =>
      setWidth(Math.max(360, entries[0].contentRect.width)),
    );
    observer.observe(scene.current);
    return () => observer.disconnect();
  }, [scene]);

  return { width, systemLight };
}
