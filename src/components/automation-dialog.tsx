"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import {
  LuCircleStop,
  LuImage,
  LuPencil,
  LuPlay,
  LuTrash2,
} from "react-icons/lu";
import { AutomationRunDialog } from "@/components/automation-run-dialog";
import { AutomationScenarioDialog } from "@/components/automation-scenario-dialog";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import {
  AnimatedTabs,
  AnimatedTabsContent,
  AnimatedTabsList,
  AnimatedTabsTrigger,
} from "@/components/ui/animated-tabs";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Progress } from "@/components/ui/progress";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useAutomationEvents } from "@/hooks/use-automation-events";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import type {
  AutomationProfileRun,
  AutomationProfileRunStatus,
  AutomationRun,
  AutomationScenario,
  BrowserProfile,
  GroupWithCount,
} from "@/types";

interface AutomationDialogProps {
  isOpen: boolean;
  onClose: () => void;
  subPage?: boolean;
  profiles: BrowserProfile[];
  groups: GroupWithCount[];
  runningProfiles: Set<string>;
}

const PROFILE_STATUS_STYLES: Record<AutomationProfileRunStatus, string> = {
  pending: "bg-muted text-muted-foreground",
  running: "bg-primary/10 text-primary",
  completed: "bg-success/10 text-success",
  failed: "bg-destructive/10 text-destructive",
  cancelled: "bg-warning/10 text-warning",
};

/** Seconds remaining on a dwell, or null when the profile isn't waiting. */
function useCountdown(waitingUntil: number | undefined): number | null {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));

  useEffect(() => {
    if (waitingUntil === undefined) return;
    const timer = setInterval(() => {
      setNow(Math.floor(Date.now() / 1000));
    }, 1000);
    return () => {
      clearInterval(timer);
    };
  }, [waitingUntil]);

  if (waitingUntil === undefined) return null;
  return Math.max(0, waitingUntil - now);
}

function ProfileRunRow({ run }: { run: AutomationProfileRun }) {
  const { t } = useTranslation();
  const remaining = useCountdown(run.waiting_until);
  const completedSteps =
    run.status === "completed"
      ? run.total_steps
      : (run.current_step_index ?? 0);
  const percent =
    run.total_steps === 0 ? 0 : (completedSteps / run.total_steps) * 100;

  return (
    <div className="flex flex-col gap-1.5 border-b px-3 py-2.5 last:border-b-0">
      <div className="flex items-center justify-between gap-2">
        <span className="truncate text-sm font-medium">{run.profile_name}</span>
        <span
          className={cn(
            "shrink-0 rounded-full px-2 py-0.5 text-xs",
            PROFILE_STATUS_STYLES[run.status],
          )}
        >
          {t(`automation.runs.profileStatus.${run.status}`)}
        </span>
      </div>

      <Progress value={percent} className="h-1" />

      <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
        <span className="truncate">
          {run.current_step_kind
            ? t("automation.runs.step", {
                current: (run.current_step_index ?? 0) + 1,
                total: run.total_steps,
                name: t(`automation.steps.${run.current_step_kind}`),
              })
            : t("automation.runs.notStarted")}
          {remaining !== null && remaining > 0
            ? ` · ${t("automation.runs.waiting", { seconds: remaining })}`
            : ""}
        </span>
        {run.screenshots.length > 0 && (
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="flex shrink-0 items-center gap-1">
                <LuImage className="size-3" />
                {run.screenshots.length}
              </span>
            </TooltipTrigger>
            <TooltipContent className="max-w-md break-all">
              {run.screenshots.join("\n")}
            </TooltipContent>
          </Tooltip>
        )}
      </div>

      {run.error && (
        <p className="text-xs text-destructive break-words">{run.error}</p>
      )}
    </div>
  );
}

function RunCard({
  run,
  onCancel,
}: {
  run: AutomationRun;
  onCancel: (runId: string) => void;
}) {
  const { t } = useTranslation();
  const finished = run.profiles.filter((p) =>
    ["completed", "failed", "cancelled"].includes(p.status),
  ).length;

  return (
    <div className="rounded-md border">
      <div className="flex items-center justify-between gap-2 border-b bg-muted/30 px-3 py-2">
        <div className="flex min-w-0 flex-col">
          <span className="truncate text-sm font-medium">
            {run.scenario_name}
          </span>
          <span className="text-xs text-muted-foreground">
            {t("automation.runs.summary", {
              finished,
              total: run.profiles.length,
              concurrency: run.concurrency,
            })}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Badge variant={run.status === "running" ? "default" : "secondary"}>
            {t(`automation.runs.status.${run.status}`)}
          </Badge>
          {run.status === "running" && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                onCancel(run.id);
              }}
            >
              <LuCircleStop className="size-4" />
              {t("automation.runs.cancelRun")}
            </Button>
          )}
        </div>
      </div>
      <div className="divide-y">
        {run.profiles.map((profileRun) => (
          <ProfileRunRow key={profileRun.profile_id} run={profileRun} />
        ))}
      </div>
    </div>
  );
}

export function AutomationDialog({
  isOpen,
  onClose,
  subPage,
  profiles,
  groups,
  runningProfiles,
}: AutomationDialogProps) {
  const { t } = useTranslation();
  const { scenarios, runs, loadScenarios, loadRuns } = useAutomationEvents();

  const [editorOpen, setEditorOpen] = useState(false);
  const [editing, setEditing] = useState<AutomationScenario | null>(null);
  const [runTarget, setRunTarget] = useState<AutomationScenario | null>(null);
  const [deleting, setDeleting] = useState<AutomationScenario | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  const activeRuns = useMemo(
    () => runs.filter((run) => run.status === "running").length,
    [runs],
  );

  const handleDelete = useCallback(async () => {
    if (!deleting) return;
    setIsDeleting(true);
    try {
      await invoke("delete_automation_scenario", { scenarioId: deleting.id });
      showSuccessToast(t("automation.scenarioDeleted"));
      await loadScenarios();
      setDeleting(null);
    } catch (err) {
      showErrorToast(translateBackendError(t, err));
    } finally {
      setIsDeleting(false);
    }
  }, [deleting, loadScenarios, t]);

  const handleCancelRun = useCallback(
    (runId: string) => {
      void (async () => {
        try {
          await invoke("cancel_automation_run", { runId });
          showSuccessToast(t("automation.runs.cancelRequested"));
        } catch (err) {
          showErrorToast(translateBackendError(t, err));
        }
      })();
    },
    [t],
  );

  /** Fetch the authoritative copy before editing, in case MCP changed it. */
  const openEditor = useCallback(
    async (scenario: AutomationScenario | null) => {
      if (!scenario) {
        setEditing(null);
        setEditorOpen(true);
        return;
      }
      try {
        setEditing(
          await invoke<AutomationScenario>("get_automation_scenario", {
            scenarioId: scenario.id,
          }),
        );
      } catch {
        setEditing(scenario);
      }
      setEditorOpen(true);
    },
    [],
  );

  return (
    <>
      <Dialog open={isOpen} onOpenChange={onClose} subPage={subPage}>
        <DialogContent className="flex max-h-[85vh] max-w-[min(72rem,calc(100%-4rem))] flex-col">
          {!subPage && (
            <DialogHeader>
              <DialogTitle>{t("automation.title")}</DialogTitle>
              <DialogDescription>
                {t("automation.description")}
              </DialogDescription>
            </DialogHeader>
          )}

          <div className="@container flex min-h-0 w-full flex-1 flex-col">
            <AnimatedTabs
              defaultValue="scenarios"
              className="flex min-h-0 flex-1 flex-col"
            >
              <div className="flex shrink-0 flex-wrap items-center justify-between gap-2">
                <AnimatedTabsList>
                  <AnimatedTabsTrigger value="scenarios">
                    <span>{t("automation.tabScenarios")}</span>
                    <span className="text-xs text-muted-foreground tabular-nums">
                      {scenarios.length}
                    </span>
                  </AnimatedTabsTrigger>
                  <AnimatedTabsTrigger value="runs">
                    <span>{t("automation.tabRuns")}</span>
                    <span className="text-xs text-muted-foreground tabular-nums">
                      {activeRuns}
                    </span>
                  </AnimatedTabsTrigger>
                </AnimatedTabsList>
                <Button
                  size="sm"
                  onClick={() => {
                    void openEditor(null);
                  }}
                >
                  <GoPlus className="size-4" />
                  {t("automation.newScenario")}
                </Button>
              </div>

              <AnimatedTabsContent
                value="scenarios"
                className="mt-4 min-h-0 flex-1 overflow-y-auto"
              >
                {scenarios.length === 0 ? (
                  <p className="rounded-md border border-dashed p-6 text-center text-sm text-muted-foreground">
                    {t("automation.emptyScenarios")}
                  </p>
                ) : (
                  <div className="flex flex-col gap-2">
                    {scenarios.map((scenario) => (
                      <div
                        key={scenario.id}
                        className="flex items-center justify-between gap-3 rounded-md border px-3 py-2.5"
                      >
                        <div className="flex min-w-0 flex-col">
                          <div className="flex items-center gap-2">
                            <span className="truncate text-sm font-medium">
                              {scenario.name}
                            </span>
                            {scenario.built_in && (
                              <Badge variant="secondary">
                                {t("automation.builtIn")}
                              </Badge>
                            )}
                          </div>
                          <span className="truncate text-xs text-muted-foreground">
                            {scenario.description ??
                              t("automation.stepCount", {
                                count: scenario.steps.length,
                              })}
                          </span>
                        </div>
                        <div className="flex shrink-0 items-center gap-1">
                          <Button
                            size="sm"
                            onClick={() => {
                              setRunTarget(scenario);
                            }}
                          >
                            <LuPlay className="size-4" />
                            {t("common.buttons.start")}
                          </Button>
                          <Tooltip>
                            <TooltipTrigger asChild>
                              <Button
                                variant="ghost"
                                size="icon"
                                aria-label={t("common.buttons.edit")}
                                onClick={() => {
                                  void openEditor(scenario);
                                }}
                              >
                                <LuPencil className="size-4" />
                              </Button>
                            </TooltipTrigger>
                            <TooltipContent>
                              {t("common.buttons.edit")}
                            </TooltipContent>
                          </Tooltip>
                          <Tooltip>
                            <TooltipTrigger asChild>
                              <Button
                                variant="ghost"
                                size="icon"
                                aria-label={t("common.buttons.delete")}
                                onClick={() => {
                                  setDeleting(scenario);
                                }}
                              >
                                <LuTrash2 className="size-4 text-destructive" />
                              </Button>
                            </TooltipTrigger>
                            <TooltipContent>
                              {t("common.buttons.delete")}
                            </TooltipContent>
                          </Tooltip>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </AnimatedTabsContent>

              <AnimatedTabsContent
                value="runs"
                className="mt-4 min-h-0 flex-1 overflow-y-auto"
              >
                {runs.length === 0 ? (
                  <p className="rounded-md border border-dashed p-6 text-center text-sm text-muted-foreground">
                    {t("automation.emptyRuns")}
                  </p>
                ) : (
                  <div className="flex flex-col gap-3">
                    {runs.map((run) => (
                      <RunCard
                        key={run.id}
                        run={run}
                        onCancel={handleCancelRun}
                      />
                    ))}
                  </div>
                )}
              </AnimatedTabsContent>
            </AnimatedTabs>
          </div>
        </DialogContent>
      </Dialog>

      <AutomationScenarioDialog
        isOpen={editorOpen}
        onClose={() => {
          setEditorOpen(false);
        }}
        scenario={editing}
        onSaved={loadScenarios}
      />

      <AutomationRunDialog
        isOpen={runTarget !== null}
        onClose={() => {
          setRunTarget(null);
        }}
        scenario={runTarget}
        profiles={profiles}
        groups={groups}
        runningProfiles={runningProfiles}
        onStarted={loadRuns}
      />

      <DeleteConfirmationDialog
        isOpen={deleting !== null}
        onClose={() => {
          setDeleting(null);
        }}
        onConfirm={() => {
          void handleDelete();
        }}
        title={t("automation.delete.title")}
        description={t("automation.delete.description", {
          name: deleting?.name ?? "",
        })}
        confirmButtonVariant="destructive"
        isLoading={isDeleting}
      />
    </>
  );
}
