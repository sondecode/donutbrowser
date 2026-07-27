"use client";

import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  LuArrowRight,
  LuBriefcase,
  LuCookie,
  LuFolders,
  LuGithub,
  LuGlobe,
  LuHeart,
  LuNetwork,
  LuShieldCheck,
  LuTerminal,
  LuUsers,
} from "react-icons/lu";
import { Logo } from "@/components/icons/logo";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";

type WelcomeStep = "intro" | "license" | "setup";

const panelTransition = {
  type: "spring",
  stiffness: 260,
  damping: 28,
} as const;

const panelVariants = {
  enter: { opacity: 0, y: 12 },
  center: { opacity: 1, y: 0 },
  exit: { opacity: 0, y: -12 },
};

// Concrete feature list shown on the intro step, rendered as an icon grid.
const FEATURES = [
  { key: "welcome.features.items.setDefault", Icon: LuGlobe },
  { key: "welcome.features.items.proxy", Icon: LuNetwork },
  { key: "welcome.features.items.vpn", Icon: LuShieldCheck },
  { key: "welcome.features.items.profiles", Icon: LuUsers },
  { key: "welcome.features.items.api", Icon: LuTerminal },
  { key: "welcome.features.items.openSource", Icon: LuGithub },
  { key: "welcome.features.items.groups", Icon: LuFolders },
  { key: "welcome.features.items.cookies", Icon: LuCookie },
] as const;

export function WelcomeDialog({
  isOpen,
  needsSetup,
  onComplete,
}: {
  isOpen: boolean;
  /**
   * Whether this user still needs the browser-download + profile-creation flow.
   * False when they already have a profile — then the welcome and commercial-use
   * steps still show, but "continue" finishes onboarding instead of proceeding
   * to setup.
   */
  needsSetup: boolean;
  onComplete: () => void;
}) {
  const { t } = useTranslation();
  const [step, setStep] = useState<WelcomeStep>("intro");
  // Where the "skip" / "continue" affordances go: into the setup flow when a
  // browser/profile is still needed, otherwise straight to completion.
  const advanceToSetup = () => {
    if (needsSetup) setStep("setup");
    else onComplete();
  };
  return (
    <Dialog open={isOpen} onOpenChange={() => {}}>
      <DialogContent
        dismissible={false}
        className="overflow-x-hidden sm:max-w-xl"
      >
        <DialogTitle className="sr-only">{t("welcome.title")}</DialogTitle>

        <AnimatePresence mode="wait">
          {step === "intro" && (
            <motion.div
              key="intro"
              variants={panelVariants}
              initial="enter"
              animate="center"
              exit="exit"
              transition={panelTransition}
              className="flex flex-col gap-7"
            >
              <div className="flex flex-col items-center gap-4 text-center">
                <motion.div
                  initial={{ opacity: 0, scale: 0.9 }}
                  animate={{ opacity: 1, scale: 1 }}
                  transition={{ ...panelTransition, delay: 0.05 }}
                  className="text-foreground"
                >
                  <Logo className="size-12" />
                </motion.div>
                <div className="flex flex-col gap-2">
                  <h2 className="text-2xl font-semibold tracking-tight text-balance">
                    {t("welcome.title")}
                  </h2>
                  <p className="mx-auto max-w-[55ch] text-sm text-pretty text-muted-foreground">
                    {t("welcome.tagline")}
                  </p>
                </div>
              </div>

              <div className="flex flex-col gap-3">
                <p className="text-sm font-medium text-muted-foreground">
                  {t("welcome.features.title")}
                </p>
                <dl className="grid grid-cols-1 gap-x-6 gap-y-3 sm:grid-cols-2">
                  {FEATURES.map(({ key, Icon }, i) => (
                    <motion.div
                      key={key}
                      initial={{ opacity: 0, y: 8 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{
                        ...panelTransition,
                        delay: 0.12 + i * 0.04,
                      }}
                      className="flex items-center gap-2.5"
                    >
                      <Icon className="size-4 shrink-0 text-muted-foreground" />
                      <dt className="text-sm font-medium text-foreground">
                        {t(key)}
                      </dt>
                    </motion.div>
                  ))}
                </dl>
              </div>

              <div className="flex items-center justify-between">
                <Button
                  variant="ghost"
                  size="sm"
                  className="text-muted-foreground hover:text-foreground"
                  onClick={advanceToSetup}
                >
                  {t("welcome.skip")}
                </Button>
                <Button
                  size="sm"
                  className="gap-1.5"
                  onClick={() => setStep("license")}
                >
                  {t("welcome.next")}
                  <LuArrowRight className="size-4 shrink-0" />
                </Button>
              </div>
            </motion.div>
          )}

          {step === "license" && (
            <motion.div
              key="license"
              variants={panelVariants}
              initial="enter"
              animate="center"
              exit="exit"
              transition={panelTransition}
              className="flex flex-col gap-7"
            >
              <div className="flex flex-col gap-2 text-center">
                <h2 className="text-2xl font-semibold tracking-tight text-balance">
                  {t("welcome.license.title")}
                </h2>
                <p className="mx-auto max-w-[55ch] text-sm/6 text-pretty text-muted-foreground">
                  {t("welcome.license.body")}
                </p>
              </div>

              <dl className="flex flex-col gap-3">
                <div className="flex items-start gap-3 rounded-lg border p-4">
                  <LuHeart className="mt-0.5 size-4 shrink-0 text-success" />
                  <div className="flex flex-col gap-0.5 text-left">
                    <dt className="text-sm font-medium text-foreground">
                      {t("welcome.license.personalTitle")}
                    </dt>
                    <dd className="text-sm text-pretty text-muted-foreground">
                      {t("welcome.license.personalDesc")}
                    </dd>
                  </div>
                </div>
                <div className="flex items-start gap-3 rounded-lg border p-4">
                  <LuBriefcase className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                  <div className="flex flex-col gap-0.5 text-left">
                    <dt className="flex items-center gap-2 text-sm font-medium text-foreground">
                      {t("welcome.license.commercialTitle")}
                      <span className="rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary">
                        {t("welcome.license.trialBadge")}
                      </span>
                    </dt>
                    <dd className="text-sm text-pretty text-muted-foreground">
                      {t("welcome.license.commercialDesc")}
                    </dd>
                  </div>
                </div>
              </dl>

              <div className="flex items-center justify-between">
                <Button
                  variant="ghost"
                  size="sm"
                  className="text-muted-foreground hover:text-foreground"
                  onClick={advanceToSetup}
                >
                  {t("welcome.skip")}
                </Button>
                <Button
                  size="sm"
                  className="gap-1.5"
                  onClick={() => {
                    if (needsSetup) setStep("setup");
                    else onComplete();
                  }}
                >
                  {t("welcome.license.agree")}
                  <LuArrowRight className="size-4 shrink-0" />
                </Button>
              </div>
            </motion.div>
          )}

          {step === "setup" && (
            <motion.div
              key="setup"
              variants={panelVariants}
              initial="enter"
              animate="center"
              exit="exit"
              transition={panelTransition}
              className="flex flex-col items-center gap-6 text-center"
            >
              <div className="flex flex-col items-center gap-2">
                <h2 className="text-2xl font-semibold tracking-tight text-balance">
                  {t("welcome.ready.title")}
                </h2>
                <p className="max-w-[55ch] text-sm/6 text-pretty text-muted-foreground">
                  {t("welcome.ready.descReady")}
                </p>
              </div>

              <Button size="sm" className="gap-1.5" onClick={onComplete}>
                <LuArrowRight className="size-4 shrink-0" />
                {t("welcome.ready.cta")}
              </Button>
            </motion.div>
          )}
        </AnimatePresence>
      </DialogContent>
    </Dialog>
  );
}
