import { useCallback, useEffect, useRef, useState } from "react";
import { onActivityTimelineUpdated } from "../../lib/activity-timeline";
import {
  type DayIntelligenceDto,
  dayIntelligence,
  onDayIntelligenceUpdated,
} from "../../lib/day-intelligence";

export type DayIntelligenceState = {
  /** The read for `day`, or null while it is loading. */
  data: DayIntelligenceDto | null;
  loaded: boolean;
  error: string | null;
  /** Reads the day again. */
  reload: () => void;
  /**
   * Shows `data` (a finished generation, the newest state of the day it was
   * started for) over reads still in flight. Dropped once the view left that day.
   */
  replace: (data: DayIntelligenceDto) => void;
};

/**
 * The day's workstreams and reports for the "Today" grid, read again when
 * the backend reports new intelligence for that day or new captured
 * activity. A read for a day the view has left is dropped.
 */
export function useDayIntelligence(day: string): DayIntelligenceState {
  const [state, setState] = useState<{
    day: string;
    data: DayIntelligenceDto | null;
    loaded: boolean;
    error: string | null;
  }>({
    day,
    data: null,
    loaded: false,
    error: null,
  });
  const latest = useRef(0);
  const currentDay = useRef(day);
  currentDay.current = day;
  const loadCurrent = useRef<() => void>(() => {});

  useEffect(() => {
    let active = true;
    const load = async () => {
      const request = ++latest.current;
      try {
        const data = await dayIntelligence(day);
        if (active && request === latest.current) {
          setState({ day, data: data ?? null, loaded: true, error: null });
        }
      } catch (failure) {
        if (active && request === latest.current) {
          const message =
            failure && typeof failure === "object" && "message" in failure
              ? String(failure.message)
              : String(failure);
          setState((previous) => ({
            day,
            data: previous.day === day ? previous.data : null,
            loaded: true,
            error: message,
          }));
        }
      }
    };
    loadCurrent.current = () => void load();
    void load();
    const cleanups: (() => void)[] = [];
    void onDayIntelligenceUpdated((payload) => {
      if (payload.day === day) void load();
    }).then((unlisten) => (active ? cleanups.push(unlisten) : unlisten()));
    void onActivityTimelineUpdated(() => void load()).then((unlisten) =>
      active ? cleanups.push(unlisten) : unlisten(),
    );
    return () => {
      active = false;
      for (const cleanup of cleanups) cleanup();
    };
  }, [day]);

  const reload = useCallback(() => loadCurrent.current(), []);
  const replace = useCallback(
    (data: DayIntelligenceDto) => {
      if (currentDay.current !== day) return;
      latest.current++;
      setState({ day, data, loaded: true, error: null });
    },
    [day],
  );

  return state.day === day
    ? { data: state.data, loaded: state.loaded, error: state.error, reload, replace }
    : { data: null, loaded: false, error: null, reload, replace };
}
