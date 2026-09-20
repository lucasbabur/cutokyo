import {
  createContext,
  type PropsWithChildren,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import type { CommandClient } from "../contracts.js";

const CommandContext = createContext<CommandClient | null>(null);

export function CommandProvider({
  client,
  children,
}: PropsWithChildren<{ readonly client: CommandClient }>) {
  return (
    <CommandContext.Provider value={client}>{children}</CommandContext.Provider>
  );
}

export function useCommands(): CommandClient {
  const client = useContext(CommandContext);
  if (client === null) throw new Error("CommandProvider is missing.");
  return client;
}

export type Resource<T> =
  | { readonly state: "loading"; readonly data: T | null; readonly error: null }
  | { readonly state: "ready"; readonly data: T; readonly error: null }
  | { readonly state: "error"; readonly data: T | null; readonly error: Error };

export function useCommandResource<T>(
  load: () => Promise<T>,
  dependencyKey: string,
): Resource<T> & { readonly reload: () => void } {
  const [generation, setGeneration] = useState(0);
  const [resource, setResource] = useState<Resource<T>>({
    state: "loading",
    data: null,
    error: null,
  });
  const previousLoad = useRef(load);
  previousLoad.current = load;

  useEffect(() => {
    let current = true;
    setResource((previous) => ({
      state: "loading",
      data: previous.data,
      error: null,
    }));
    void previousLoad
      .current()
      .then((data) => {
        if (current) setResource({ state: "ready", data, error: null });
      })
      .catch((reason: unknown) => {
        const error =
          reason instanceof Error ? reason : new Error(String(reason));
        if (current)
          setResource((previous) => ({
            state: "error",
            data: previous.data,
            error,
          }));
      });
    return () => {
      current = false;
    };
  }, [dependencyKey, generation]);

  const reload = useCallback(() => setGeneration((value) => value + 1), []);
  return useMemo(() => ({ ...resource, reload }), [reload, resource]);
}
