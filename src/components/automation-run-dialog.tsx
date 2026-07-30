"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import type {
  AutomationScenario,
  BrowserProfile,
  GroupWithCount,
} from "@/types";

interface AutomationRunDialogProps {
  isOpen: boolean;
  onClose: () => void;
  scenario: AutomationScenario | null;
  profiles: BrowserProfile[];
  groups: GroupWithCount[];
  runningProfiles: Set<string>;
  onStarted: () => void | Promise<void>;
}

/** Only Chromium-family profiles expose the CDP endpoint automation drives. */
const CHROMIUM_BROWSERS = new Set(["chromium", "wayfern"]);

type TargetMode = "individual" | "group";

export function AutomationRunDialog({
  isOpen,
  onClose,
  scenario,
  profiles,
  groups,
  runningProfiles,
  onStarted,
}: AutomationRunDialogProps) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<TargetMode>("individual");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [groupId, setGroupId] = useState<string | null>(null);
  const [concurrency, setConcurrency] = useState("1");
  const [jitterMin, setJitterMin] = useState("0");
  const [jitterMax, setJitterMax] = useState("0");
  const [headless, setHeadless] = useState(false);
  const [variables, setVariables] = useState<Record<string, string>>({});
  const [isStarting, setIsStarting] = useState(false);

  const eligibleProfiles = useMemo(
    () =>
      profiles.filter(
        (profile) =>
          CHROMIUM_BROWSERS.has(profile.browser.trim().toLowerCase()) &&
          !runningProfiles.has(profile.id),
      ),
    [profiles, runningProfiles],
  );

  /**
   * Eligible members per group. The backend skips ineligible members of a group
   * rather than failing, so this count is what will actually run — showing the
   * group's raw total would over-promise.
   */
  const eligibleCountByGroup = useMemo(() => {
    const counts = new Map<string, number>();
    for (const profile of eligibleProfiles) {
      if (!profile.group_id) continue;
      counts.set(profile.group_id, (counts.get(profile.group_id) ?? 0) + 1);
    }
    return counts;
  }, [eligibleProfiles]);

  const groupEligibleCount = groupId
    ? (eligibleCountByGroup.get(groupId) ?? 0)
    : 0;

  const targetCount = mode === "group" ? groupEligibleCount : selected.size;

  useEffect(() => {
    if (!isOpen) return;
    setMode("individual");
    setSelected(new Set());
    setGroupId(null);
    setConcurrency("1");
    setJitterMin("0");
    setJitterMax("0");
    setHeadless(false);
    setVariables(
      Object.fromEntries(
        (scenario?.variables ?? []).map((variable) => [
          variable.name,
          variable.default ?? "",
        ]),
      ),
    );
  }, [isOpen, scenario]);

  const toggleProfile = useCallback((profileId: string) => {
    setSelected((previous) => {
      const next = new Set(previous);
      if (next.has(profileId)) next.delete(profileId);
      else next.add(profileId);
      return next;
    });
  }, []);

  const toggleAll = useCallback(() => {
    setSelected((previous) =>
      previous.size === eligibleProfiles.length
        ? new Set()
        : new Set(eligibleProfiles.map((profile) => profile.id)),
    );
  }, [eligibleProfiles]);

  const handleStart = useCallback(async () => {
    if (!scenario) return;
    setIsStarting(true);
    try {
      await invoke("start_automation_run", {
        request: {
          scenario_id: scenario.id,
          // Send exactly one target kind, so the backend's per-kind semantics
          // (fail on an explicit pick vs skip a group member) stay predictable.
          profile_ids: mode === "individual" ? [...selected] : [],
          group_id: mode === "group" ? groupId : null,
          concurrency: Math.max(1, Number.parseInt(concurrency, 10) || 1),
          jitter_min_secs: Math.max(0, Number.parseInt(jitterMin, 10) || 0),
          jitter_max_secs: Math.max(0, Number.parseInt(jitterMax, 10) || 0),
          variables,
          headless,
        },
      });
      showSuccessToast(t("automation.run.started", { count: targetCount }));
      await onStarted();
      onClose();
    } catch (err) {
      showErrorToast(translateBackendError(t, err));
    } finally {
      setIsStarting(false);
    }
  }, [
    scenario,
    mode,
    selected,
    groupId,
    targetCount,
    concurrency,
    jitterMin,
    jitterMax,
    variables,
    headless,
    onStarted,
    onClose,
    t,
  ]);

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="flex max-h-[85vh] max-w-2xl flex-col">
        <DialogHeader>
          <DialogTitle>
            {t("automation.run.title", { name: scenario?.name ?? "" })}
          </DialogTitle>
          <DialogDescription>
            {t("automation.run.description")}
          </DialogDescription>
        </DialogHeader>

        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto">
          {(scenario?.variables ?? []).length > 0 && (
            <div className="flex flex-col gap-3">
              <Label>{t("automation.run.variablesLabel")}</Label>
              {(scenario?.variables ?? []).map((variable) => (
                <div key={variable.name} className="flex flex-col gap-1">
                  <Label
                    htmlFor={`automation-var-${variable.name}`}
                    className="font-mono text-xs text-muted-foreground"
                  >
                    {variable.name}
                  </Label>
                  <Input
                    id={`automation-var-${variable.name}`}
                    value={variables[variable.name] ?? ""}
                    onChange={(e) => {
                      setVariables((previous) => ({
                        ...previous,
                        [variable.name]: e.target.value,
                      }));
                    }}
                    placeholder={variable.description ?? ""}
                    disabled={isStarting}
                  />
                </div>
              ))}
            </div>
          )}

          <div className="flex flex-col gap-2">
            <div className="flex items-center justify-between gap-2">
              <Label>{t("automation.run.targetLabel")}</Label>
              {mode === "individual" && eligibleProfiles.length > 0 && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={toggleAll}
                  disabled={isStarting}
                >
                  {selected.size === eligibleProfiles.length
                    ? t("automation.run.clearSelection")
                    : t("automation.run.selectAll")}
                </Button>
              )}
            </div>

            <div className="flex w-full rounded-md border p-0.5">
              {(["individual", "group"] as const).map((option) => (
                <button
                  key={option}
                  type="button"
                  onClick={() => {
                    setMode(option);
                  }}
                  disabled={isStarting}
                  className={cn(
                    "flex-1 rounded-sm px-3 py-1.5 text-sm transition-colors",
                    mode === option
                      ? "bg-primary text-primary-foreground"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  {t(`automation.run.mode.${option}`)}
                </button>
              ))}
            </div>

            {mode === "individual" ? (
              <>
                <p className="text-xs text-muted-foreground">
                  {t("automation.run.profilesHint")}
                </p>
                {eligibleProfiles.length === 0 ? (
                  <p className="rounded-md border border-dashed p-4 text-center text-sm text-muted-foreground">
                    {t("automation.run.noEligibleProfiles")}
                  </p>
                ) : (
                  <div className="max-h-56 overflow-y-auto rounded-md border">
                    {eligibleProfiles.map((profile) => (
                      <label
                        key={profile.id}
                        htmlFor={`automation-profile-${profile.id}`}
                        className="flex cursor-pointer items-center gap-3 border-b px-3 py-2 last:border-b-0 hover:bg-accent/40"
                      >
                        <Checkbox
                          id={`automation-profile-${profile.id}`}
                          checked={selected.has(profile.id)}
                          onCheckedChange={() => {
                            toggleProfile(profile.id);
                          }}
                          disabled={isStarting}
                        />
                        <span className="truncate text-sm">{profile.name}</span>
                      </label>
                    ))}
                  </div>
                )}
              </>
            ) : (
              <>
                <p className="text-xs text-muted-foreground">
                  {t("automation.run.groupHint")}
                </p>
                {groups.length === 0 ? (
                  <p className="rounded-md border border-dashed p-4 text-center text-sm text-muted-foreground">
                    {t("automation.run.noGroups")}
                  </p>
                ) : (
                  <>
                    <Select
                      value={groupId ?? ""}
                      onValueChange={setGroupId}
                      disabled={isStarting}
                    >
                      <SelectTrigger>
                        <SelectValue
                          placeholder={t("automation.run.groupPlaceholder")}
                        />
                      </SelectTrigger>
                      <SelectContent>
                        {groups.map((group) => (
                          <SelectItem key={group.id} value={group.id}>
                            {t("automation.run.groupOption", {
                              name: group.name,
                              count: eligibleCountByGroup.get(group.id) ?? 0,
                            })}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    {groupId !== null && groupEligibleCount === 0 && (
                      <p className="text-xs text-warning">
                        {t("automation.run.groupHasNoEligible")}
                      </p>
                    )}
                  </>
                )}
              </>
            )}
          </div>

          <div className="grid gap-4 @sm:grid-cols-2">
            <div className="flex flex-col gap-2">
              <Label htmlFor="automation-concurrency">
                {t("automation.run.concurrencyLabel")}
              </Label>
              <Input
                id="automation-concurrency"
                type="number"
                min={1}
                value={concurrency}
                onChange={(e) => {
                  setConcurrency(e.target.value);
                }}
                disabled={isStarting}
              />
              <p className="text-xs text-muted-foreground">
                {t("automation.run.concurrencyHint")}
              </p>
            </div>

            <div className="flex flex-col gap-2">
              <Label htmlFor="automation-jitter-min">
                {t("automation.run.jitterLabel")}
              </Label>
              <div className="flex items-center gap-2">
                <Input
                  id="automation-jitter-min"
                  type="number"
                  min={0}
                  value={jitterMin}
                  onChange={(e) => {
                    setJitterMin(e.target.value);
                  }}
                  disabled={isStarting}
                />
                <span className="text-muted-foreground">–</span>
                <Input
                  aria-label={t("automation.run.jitterMaxAria")}
                  type="number"
                  min={0}
                  value={jitterMax}
                  onChange={(e) => {
                    setJitterMax(e.target.value);
                  }}
                  disabled={isStarting}
                />
              </div>
              <p className="text-xs text-muted-foreground">
                {t("automation.run.jitterHint")}
              </p>
            </div>
          </div>

          <div className="flex items-center justify-between rounded-md border p-3">
            <div className="flex flex-col">
              <Label htmlFor="automation-headless">
                {t("automation.run.headlessLabel")}
              </Label>
              <p className="text-xs text-muted-foreground">
                {t("automation.run.headlessHint")}
              </p>
            </div>
            <AnimatedSwitch
              id="automation-headless"
              checked={headless}
              onCheckedChange={setHeadless}
              disabled={isStarting}
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={isStarting}>
            {t("common.buttons.cancel")}
          </Button>
          <Button
            onClick={() => {
              void handleStart();
            }}
            disabled={isStarting || targetCount === 0}
          >
            {t("automation.run.start", { count: targetCount })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
