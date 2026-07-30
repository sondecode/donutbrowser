"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import {
  LuChevronDown,
  LuChevronRight,
  LuCircleStop,
  LuImage,
  LuPencil,
  LuPlay,
  LuTrash2,
} from "react-icons/lu";
import { AutomationRunDialog } from "@/components/automation-run-dialog";
import { AutomationScenarioDialog } from "@/components/automation-scenario-dialog";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
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
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
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

function ProfileRunRow({
  run,
  onOpenScreenshot,
}: {
  run: AutomationProfileRun;
  onOpenScreenshot: (path: string) => void;
}) {
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
      </div>

      {run.screenshots.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {run.screenshots.map((path, index) => (
            <Tooltip key={path}>
              <TooltipTrigger asChild>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-7 gap-1.5 px-2 text-xs"
                  onClick={() => {
                    onOpenScreenshot(path);
                  }}
                >
                  <LuImage className="size-3.5" />
                  {t("automation.runs.screenshot", { index: index + 1 })}
                </Button>
              </TooltipTrigger>
              <TooltipContent className="max-w-md break-all">
                {path}
              </TooltipContent>
            </Tooltip>
          ))}
        </div>
      )}

      {run.error && (
        <p className="text-xs text-destructive break-words">{run.error}</p>
      )}
    </div>
  );
}

function RunHistoryItem({
  run,
  expanded,
  onToggle,
  onCancel,
  onDelete,
  onOpenScreenshot,
}: {
  run: AutomationRun;
  expanded: boolean;
  onToggle: (runId: string) => void;
  onCancel: (runId: string) => void;
  onDelete: (run: AutomationRun) => void;
  onOpenScreenshot: (path: string) => void;
}) {
  const { t } = useTranslation();
  const finished = run.profiles.filter((p) =>
    ["completed", "failed", "cancelled"].includes(p.status),
  ).length;

  return (
    <div className="rounded-md border">
      <div className="flex items-center justify-between gap-2 bg-muted/30 px-3 py-2">
        <div className="flex min-w-0 flex-col">
          <button
            type="button"
            className="flex min-w-0 items-center gap-2 text-left"
            onClick={() => {
              onToggle(run.id);
            }}
            aria-label={t(
              expanded
                ? "automation.runs.collapseRun"
                : "automation.runs.expandRun",
            )}
          >
            {expanded ? (
              <LuChevronDown className="size-4 shrink-0 text-muted-foreground" />
            ) : (
              <LuChevronRight className="size-4 shrink-0 text-muted-foreground" />
            )}
            <span className="truncate text-sm font-medium">
              {run.scenario_name}
            </span>
          </button>
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
          {run.status !== "running" && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("automation.runs.deleteRun")}
                  onClick={() => {
                    onDelete(run);
                  }}
                >
                  <LuTrash2 className="size-4 text-destructive" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("automation.runs.deleteRun")}</TooltipContent>
            </Tooltip>
          )}
        </div>
      </div>
      {expanded && (
        <div className="divide-y border-t">
          {run.profiles.map((profileRun) => (
            <ProfileRunRow
              key={profileRun.profile_id}
              run={profileRun}
              onOpenScreenshot={onOpenScreenshot}
            />
          ))}
        </div>
      )}
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
  const [deletingRun, setDeletingRun] = useState<AutomationRun | null>(null);
  const [deleteAllRunsOpen, setDeleteAllRunsOpen] = useState(false);
  const [isDeleting, setIsDeleting] = useState(false);
  const [isDeletingRun, setIsDeletingRun] = useState(false);
  const [isDeletingAllRuns, setIsDeletingAllRuns] = useState(false);
  const [isTogglingSync, setIsTogglingSync] = useState<Record<string, boolean>>(
    {},
  );
  const [expandedRunIds, setExpandedRunIds] = useState<Set<string>>(
    () => new Set(),
  );

  const activeRuns = useMemo(
    () => runs.filter((run) => run.status === "running").length,
    [runs],
  );
  const finishedRuns = useMemo(
    () => runs.filter((run) => run.status !== "running").length,
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

  const handleToggleScenarioSync = useCallback(
    async (scenario: AutomationScenario, enabled: boolean) => {
      setIsTogglingSync((prev) => ({ ...prev, [scenario.id]: true }));
      try {
        await invoke<AutomationScenario>(
          "set_automation_scenario_sync_enabled",
          { scenarioId: scenario.id, enabled },
        );
        showSuccessToast(
          t(
            enabled
              ? "automation.scenarioSyncEnabled"
              : "automation.scenarioSyncDisabled",
          ),
        );
        await loadScenarios();
      } catch (err) {
        showErrorToast(translateBackendError(t, err));
      } finally {
        setIsTogglingSync((prev) => {
          const next = { ...prev };
          delete next[scenario.id];
          return next;
        });
      }
    },
    [loadScenarios, t],
  );

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

  const handleOpenScreenshot = useCallback(
    (path: string) => {
      void (async () => {
        try {
          await invoke("open_automation_screenshot", { path });
        } catch (err) {
          showErrorToast(translateBackendError(t, err));
        }
      })();
    },
    [t],
  );

  const handleDeleteRun = useCallback(async () => {
    if (!deletingRun) return;
    setIsDeletingRun(true);
    try {
      await invoke("delete_automation_run", { runId: deletingRun.id });
      showSuccessToast(t("automation.runs.runDeleted"));
      setDeletingRun(null);
      await loadRuns();
    } catch (err) {
      showErrorToast(translateBackendError(t, err));
    } finally {
      setIsDeletingRun(false);
    }
  }, [deletingRun, loadRuns, t]);

  const handleDeleteAllRuns = useCallback(async () => {
    setIsDeletingAllRuns(true);
    try {
      const deleted = await invoke<number>("delete_all_automation_runs");
      showSuccessToast(t("automation.runs.allDeleted", { count: deleted }));
      setDeleteAllRunsOpen(false);
      await loadRuns();
    } catch (err) {
      showErrorToast(translateBackendError(t, err));
    } finally {
      setIsDeletingAllRuns(false);
    }
  }, [loadRuns, t]);

  const toggleRunExpanded = useCallback((runId: string) => {
    setExpandedRunIds((previous) => {
      const next = new Set(previous);
      if (next.has(runId)) {
        next.delete(runId);
      } else {
        next.add(runId);
      }
      return next;
    });
  }, []);

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
                  <Table containerClassName="rounded-md border">
                    <TableHeader>
                      <TableRow>
                        <TableHead>{t("automation.table.name")}</TableHead>
                        <TableHead>{t("automation.table.steps")}</TableHead>
                        <TableHead>{t("automation.table.sync")}</TableHead>
                        <TableHead>{t("automation.table.updated")}</TableHead>
                        <TableHead className="text-right">
                          {t("automation.table.actions")}
                        </TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {scenarios.map((scenario) => (
                        <TableRow key={scenario.id}>
                          <TableCell className="min-w-56">
                            <div className="flex min-w-0 flex-col gap-1">
                              <div className="flex min-w-0 items-center gap-2">
                                <span className="truncate font-medium">
                                  {scenario.name}
                                </span>
                                {scenario.built_in && (
                                  <Badge variant="secondary">
                                    {t("automation.builtIn")}
                                  </Badge>
                                )}
                              </div>
                              {scenario.description && (
                                <span className="max-w-md truncate text-xs text-muted-foreground">
                                  {scenario.description}
                                </span>
                              )}
                            </div>
                          </TableCell>
                          <TableCell>
                            {t("automation.stepCount", {
                              count: scenario.steps.length,
                            })}
                          </TableCell>
                          <TableCell>
                            <AnimatedSwitch
                              checked={scenario.sync_enabled ?? true}
                              disabled={isTogglingSync[scenario.id]}
                              aria-label={t("automation.table.syncScenario", {
                                name: scenario.name,
                              })}
                              onCheckedChange={(checked) => {
                                void handleToggleScenarioSync(
                                  scenario,
                                  checked,
                                );
                              }}
                            />
                          </TableCell>
                          <TableCell className="text-muted-foreground">
                            {scenario.updated_at
                              ? new Date(
                                  scenario.updated_at * 1000,
                                ).toLocaleString()
                              : t("automation.table.never")}
                          </TableCell>
                          <TableCell>
                            <div className="flex justify-end gap-1">
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
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
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
                    <div className="flex justify-end">
                      <Button
                        variant="outline"
                        size="sm"
                        disabled={finishedRuns === 0}
                        onClick={() => {
                          setDeleteAllRunsOpen(true);
                        }}
                      >
                        <LuTrash2 className="size-4" />
                        {t("automation.runs.deleteAll")}
                      </Button>
                    </div>
                    {runs.map((run) => (
                      <RunHistoryItem
                        key={run.id}
                        run={run}
                        expanded={expandedRunIds.has(run.id)}
                        onToggle={toggleRunExpanded}
                        onCancel={handleCancelRun}
                        onDelete={setDeletingRun}
                        onOpenScreenshot={handleOpenScreenshot}
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

      <DeleteConfirmationDialog
        isOpen={deletingRun !== null}
        onClose={() => {
          setDeletingRun(null);
        }}
        onConfirm={() => {
          void handleDeleteRun();
        }}
        title={t("automation.runs.deleteRunTitle")}
        description={t("automation.runs.deleteRunDescription", {
          name: deletingRun?.scenario_name ?? "",
        })}
        confirmButtonVariant="destructive"
        isLoading={isDeletingRun}
      />

      <DeleteConfirmationDialog
        isOpen={deleteAllRunsOpen}
        onClose={() => {
          setDeleteAllRunsOpen(false);
        }}
        onConfirm={() => {
          void handleDeleteAllRuns();
        }}
        title={t("automation.runs.deleteAllTitle")}
        description={t("automation.runs.deleteAllDescription")}
        confirmButtonVariant="destructive"
        isLoading={isDeletingAllRuns}
      />
    </>
  );
}
