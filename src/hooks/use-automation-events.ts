import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import type { AutomationRun, AutomationScenario } from "@/types";

interface UseAutomationEventsReturn {
  scenarios: AutomationScenario[];
  runs: AutomationRun[];
  isLoading: boolean;
  loadScenarios: () => Promise<void>;
  loadRuns: () => Promise<void>;
}

/**
 * Scenario list plus live run progress.
 *
 * Runs arrive through the `automation-run-updated` event while the page is
 * mounted; `loadRuns` covers the initial catch-up for runs already in flight.
 */
export function useAutomationEvents(): UseAutomationEventsReturn {
  const [scenarios, setScenarios] = useState<AutomationScenario[]>([]);
  const [runs, setRuns] = useState<AutomationRun[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  const loadScenarios = useCallback(async () => {
    try {
      setScenarios(
        await invoke<AutomationScenario[]>("list_automation_scenarios"),
      );
    } catch (err) {
      console.error("Failed to load automation scenarios:", err);
    }
  }, []);

  const loadRuns = useCallback(async () => {
    try {
      setRuns(await invoke<AutomationRun[]>("list_automation_runs"));
    } catch (err) {
      console.error("Failed to load automation runs:", err);
    }
  }, []);

  useEffect(() => {
    void (async () => {
      await Promise.all([loadScenarios(), loadRuns()]);
      setIsLoading(false);
    })();
  }, [loadScenarios, loadRuns]);

  useEffect(() => {
    const unlisten = listen<AutomationRun>(
      "automation-run-updated",
      (event) => {
        const updated = event.payload;
        setRuns((previous) => {
          const index = previous.findIndex((run) => run.id === updated.id);
          if (index === -1) return [updated, ...previous];
          const next = [...previous];
          next[index] = updated;
          return next;
        });
      },
    );

    return () => {
      void unlisten.then((fn) => {
        fn();
      });
    };
  }, []);

  return { scenarios, runs, isLoading, loadScenarios, loadRuns };
}
