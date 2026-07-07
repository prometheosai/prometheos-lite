"use client"

import { useState } from "react"
import { AlertTriangle, Shield, Check, X, AlertCircle } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { cn } from "@/lib/utils"

interface ToolPermissionRequest {
  toolName: string
  requestedAction: string
  riskLevel: "low" | "medium" | "high"
  reason?: string
  onAllow: () => void
  onDeny: () => void
  onAlwaysAllow?: () => void
}

interface ToolPermissionModalProps {
  isOpen: boolean
  request: ToolPermissionRequest | null
  onClose: () => void
}

export function ToolPermissionModal({ isOpen, request, onClose }: ToolPermissionModalProps) {
  const getRiskColor = (level: string) => {
    switch (level) {
      case "low":
        return "bg-green-900/20 border-green-800/50 text-green-200"
      case "medium":
        return "bg-yellow-900/20 border-yellow-800/50 text-yellow-200"
      case "high":
        return "bg-red-900/20 border-red-800/50 text-red-200"
      default:
        return "bg-muted border-border"
    }
  }

  const getRiskIcon = (level: string) => {
    switch (level) {
      case "low":
        return <Check className="h-4 w-4" />
      case "medium":
        return <AlertTriangle className="h-4 w-4" />
      case "high":
        return <AlertCircle className="h-4 w-4" />
      default:
        return <Shield className="h-4 w-4" />
    }
  }

  if (!request) return null

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Shield className="h-5 w-5" />
            Tool Permission Request
          </DialogTitle>
          <DialogDescription>
            A tool is requesting permission to execute an action. Review the details below.
          </DialogDescription>
        </DialogHeader>
        
        <div className="space-y-4 py-4">
          <div className="flex items-center justify-between p-3 rounded-lg border">
            <div className="flex items-center gap-2">
              <span className="font-medium">Tool:</span>
              <span className="text-muted-foreground">{request.toolName}</span>
            </div>
            <div className={cn(
              "flex items-center gap-1.5 px-2 py-1 rounded-full text-xs font-medium border",
              getRiskColor(request.riskLevel)
            )}>
              {getRiskIcon(request.riskLevel)}
              {request.riskLevel.charAt(0).toUpperCase() + request.riskLevel.slice(1)} Risk
            </div>
          </div>

          <div className="p-3 rounded-lg bg-muted/50 border">
            <div className="font-medium mb-1">Requested Action:</div>
            <div className="text-sm text-muted-foreground">{request.requestedAction}</div>
          </div>

          {request.reason && (
            <div className="p-3 rounded-lg bg-muted/50 border">
              <div className="font-medium mb-1 flex items-center gap-2">
                <Shield className="h-4 w-4" />
                Policy Reason:
              </div>
              <div className="text-sm text-muted-foreground">{request.reason}</div>
            </div>
          )}
        </div>

        <DialogFooter className="flex-col sm:flex-row gap-2">
          <Button
            onClick={() => {
              request.onAllow()
              onClose()
            }}
            className="w-full sm:w-auto bg-primary text-primary-foreground"
          >
            <Check className="h-4 w-4 mr-2" />
            Allow
          </Button>
          {request.onAlwaysAllow && (
            <Button
              onClick={() => {
                request.onAlwaysAllow!()
                onClose()
              }}
              variant="outline"
              className="w-full sm:w-auto"
            >
              <Shield className="h-4 w-4 mr-2" />
              Always Allow
            </Button>
          )}
          <Button
            onClick={() => {
              request.onDeny()
              onClose()
            }}
            variant="destructive"
            className="w-full sm:w-auto"
          >
            <X className="h-4 w-4 mr-2" />
            Deny
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
