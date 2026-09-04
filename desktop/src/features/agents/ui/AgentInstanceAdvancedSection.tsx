import type { ComponentProps } from "react";
import { ChevronDown } from "lucide-react";
import { AnimatePresence, motion, type Transition } from "motion/react";

import { cn } from "@/shared/lib/cn";
import { AdvancedRequiredBadge } from "./AdvancedRequiredBadge";
import { EditAgentAdvancedFields } from "./EditAgentAdvancedFields";

export function AgentInstanceAdvancedSection({
  disabled,
  envVars,
  fieldProps,
  onOpenChange,
  open,
  requiredEnvKeys,
  transition,
}: {
  disabled: boolean;
  envVars: Record<string, string>;
  fieldProps: ComponentProps<typeof EditAgentAdvancedFields>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  requiredEnvKeys: readonly string[];
  transition: Transition;
}) {
  return (
    <div className="space-y-3">
      <button
        aria-expanded={open}
        className="inline-flex h-9 items-center gap-1.5 text-sm font-medium text-foreground transition-colors hover:text-foreground/80 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
        disabled={disabled}
        onClick={() => onOpenChange(!open)}
        type="button"
      >
        <span>Advanced</span>
        <AdvancedRequiredBadge
          envVars={envVars}
          requiredEnvKeys={requiredEnvKeys}
          testId="edit-agent-advanced-required-badge"
        />
        <ChevronDown
          className={cn(
            "h-4 w-4 text-muted-foreground transition-transform duration-150 ease-out",
            open && "rotate-180",
          )}
        />
      </button>
      <AnimatePresence initial={false}>
        {open ? (
          <motion.div
            animate={{ height: "auto", opacity: 1, scale: 1 }}
            className="origin-top overflow-hidden"
            exit={{ height: 0, opacity: 0, scale: 0.98 }}
            initial={{ height: 0, opacity: 0, scale: 0.98 }}
            key="edit-agent-advanced-fields"
            transition={transition}
          >
            <EditAgentAdvancedFields {...fieldProps} />
          </motion.div>
        ) : null}
      </AnimatePresence>
    </div>
  );
}
