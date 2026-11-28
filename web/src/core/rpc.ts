// The page's side of the worker (worker/index.ts): typed calls that answer with a promise, and
// the events the worker sends on its own.
import type { Calls, Op, Reply, WorkerEvent } from "./protocol";

type EventName = WorkerEvent["event"];
type Listener<E extends EventName> = (event: Extract<WorkerEvent, { event: E }>) => void;

const worker = new Worker(new URL("../worker/index.ts", import.meta.url), { type: "module" });
const waiting = new Map<number, { resolve: (value: unknown) => void; reject: (error: Error) => void }>();
const listeners = new Map<EventName, Set<(event: WorkerEvent) => void>>();
let next = 0;

/** Ask the worker for `op`. */
export function call<K extends Op>(op: K, args: Calls[K]["args"]): Promise<Calls[K]["result"]> {
  const id = next++;
  worker.postMessage({ id, op, args });
  return new Promise((resolve, reject) => waiting.set(id, { resolve: resolve as (value: unknown) => void, reject }));
}

/** Listen for the worker's `name` events; returns the function that stops listening. */
export function on<E extends EventName>(name: E, listener: Listener<E>): () => void {
  let set = listeners.get(name);
  if (!set) listeners.set(name, (set = new Set()));
  const any = listener as (event: WorkerEvent) => void;
  set.add(any);
  return () => set.delete(any);
}

worker.onmessage = ({ data }: MessageEvent<Reply>) => {
  if ("event" in data) {
    listeners.get(data.event)?.forEach((listener) => listener(data));
    return;
  }
  const waiter = waiting.get(data.id);
  waiting.delete(data.id);
  if ("error" in data) waiter?.reject(new Error(data.error));
  else waiter?.resolve(data.result);
};
