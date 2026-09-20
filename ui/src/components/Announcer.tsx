import {
  createContext,
  type PropsWithChildren,
  useCallback,
  useContext,
  useState,
} from "react";

const AnnouncementContext = createContext<(message: string) => void>(
  () => undefined,
);

export function AnnouncementProvider({ children }: PropsWithChildren) {
  const [message, setMessage] = useState("");
  const announce = useCallback((next: string) => {
    setMessage("");
    globalThis.setTimeout(() => setMessage(next), 20);
  }, []);
  return (
    <AnnouncementContext.Provider value={announce}>
      {children}
      <div
        className="sr-only"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        {message}
      </div>
    </AnnouncementContext.Provider>
  );
}

export function useAnnounce(): (message: string) => void {
  return useContext(AnnouncementContext);
}
