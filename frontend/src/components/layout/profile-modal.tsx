"use client"

import { useState } from "react"
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import { Avatar, AvatarFallback } from "@/components/ui/avatar"
import { useTheme } from "next-themes"
import { LogOut, Settings, User as UserIcon, Cpu, Brain, Shield, CreditCard, Moon, Sun } from "lucide-react"

interface ProfileModalProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function ProfileModal({ open, onOpenChange }: ProfileModalProps) {
  const { theme, setTheme } = useTheme()
  const [activeSection, setActiveSection] = useState<"account" | "preferences" | "models" | "memory" | "privacy" | "billing">("account")

  const sections = [
    { id: "account", label: "Account", icon: UserIcon },
    { id: "preferences", label: "Preferences", icon: Settings },
    { id: "models", label: "Models", icon: Cpu },
    { id: "memory", label: "Memory", icon: Brain },
    { id: "privacy", label: "Privacy", icon: Shield },
    { id: "billing", label: "Billing", icon: CreditCard },
  ] as const

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Profile & Settings</DialogTitle>
        </DialogHeader>

        <div className="flex gap-6 mt-4">
          {/* Sidebar */}
          <div className="w-48 space-y-1">
            {sections.map((section) => {
              const Icon = section.icon
              return (
                <Button
                  key={section.id}
                  variant={activeSection === section.id ? "secondary" : "ghost"}
                  className="w-full justify-start"
                  onClick={() => setActiveSection(section.id)}
                >
                  <Icon className="h-4 w-4 mr-2" />
                  {section.label}
                </Button>
              )
            })}
            <Separator className="my-2" />
            <Button variant="ghost" className="w-full justify-start text-destructive hover:text-destructive">
              <LogOut className="h-4 w-4 mr-2" />
              Logout
            </Button>
          </div>

          {/* Content */}
          <div className="flex-1">
            {activeSection === "account" && (
              <div className="space-y-4">
                <h3 className="text-lg font-semibold">Account</h3>
                <div className="flex items-center gap-4">
                  <Avatar className="h-16 w-16">
                    <AvatarFallback className="text-lg">DR</AvatarFallback>
                  </Avatar>
                  <div>
                    <div className="font-semibold text-lg">Diego Rhoger</div>
                    <div className="text-sm text-muted-foreground">diego@example.com</div>
                  </div>
                </div>
              </div>
            )}

            {activeSection === "preferences" && (
              <div className="space-y-4">
                <h3 className="text-lg font-semibold">Preferences</h3>
                <div className="space-y-4">
                  <div className="flex items-center justify-between">
                    <div>
                      <div className="font-medium">Theme</div>
                      <div className="text-sm text-muted-foreground">Choose your preferred theme</div>
                    </div>
                    <div className="flex gap-2">
                      <Button
                        variant={theme === "light" ? "default" : "outline"}
                        size="icon"
                        onClick={() => setTheme("light")}
                      >
                        <Sun className="h-4 w-4" />
                      </Button>
                      <Button
                        variant={theme === "dark" ? "default" : "outline"}
                        size="icon"
                        onClick={() => setTheme("dark")}
                      >
                        <Moon className="h-4 w-4" />
                      </Button>
                    </div>
                  </div>
                  <Separator />
                  <div className="flex items-center justify-between opacity-50">
                    <div>
                      <div className="font-medium">Default Model</div>
                      <div className="text-sm text-muted-foreground">Select your default AI model</div>
                    </div>
                    <Button variant="outline" disabled>
                      LM Studio
                    </Button>
                  </div>
                  <Separator />
                  <div className="flex items-center justify-between opacity-50">
                    <div>
                      <div className="font-medium">Response Style</div>
                      <div className="text-sm text-muted-foreground">Choose response format</div>
                    </div>
                    <Button variant="outline" disabled>
                      Balanced
                    </Button>
                  </div>
                </div>
              </div>
            )}

            {activeSection === "models" && (
              <div className="space-y-4">
                <h3 className="text-lg font-semibold">Models</h3>
                <div className="space-y-4 opacity-50">
                  <div className="flex items-center justify-between">
                    <div>
                      <div className="font-medium">Active Model</div>
                      <div className="text-sm text-muted-foreground">Currently selected model</div>
                    </div>
                    <Button variant="outline" disabled>
                      LM Studio
                    </Button>
                  </div>
                  <Separator />
                  <div className="text-sm text-muted-foreground">
                    Additional model configuration coming soon.
                  </div>
                </div>
              </div>
            )}

            {activeSection === "memory" && (
              <div className="space-y-4">
                <h3 className="text-lg font-semibold">Memory Settings</h3>
                <div className="space-y-4 opacity-50">
                  <div className="flex items-center justify-between">
                    <div>
                      <div className="font-medium">Memory Retention</div>
                      <div className="text-sm text-muted-foreground">How long to keep memories</div>
                    </div>
                    <Button variant="outline" disabled>
                      30 days
                    </Button>
                  </div>
                  <Separator />
                  <div className="flex items-center justify-between">
                    <div>
                      <div className="font-medium">Auto-summarize</div>
                      <div className="text-sm text-muted-foreground">Automatically summarize long conversations</div>
                    </div>
                    <Button variant="outline" disabled>
                      Off
                    </Button>
                  </div>
                  <div className="mt-4 p-3 bg-muted rounded-md">
                    <div className="text-sm font-medium">Coming soon</div>
                    <div className="text-xs text-muted-foreground">
                      Advanced memory management features will be available in a future update.
                    </div>
                  </div>
                </div>
              </div>
            )}

            {activeSection === "privacy" && (
              <div className="space-y-4">
                <h3 className="text-lg font-semibold">Privacy</h3>
                <div className="space-y-4 opacity-50">
                  <div className="flex items-center justify-between">
                    <div>
                      <div className="font-medium">Data Sharing</div>
                      <div className="text-sm text-muted-foreground">Share anonymous usage data</div>
                    </div>
                    <Button variant="outline" disabled>
                      Off
                    </Button>
                  </div>
                  <Separator />
                  <div className="flex items-center justify-between">
                    <div>
                      <div className="font-medium">Local Storage Only</div>
                      <div className="text-sm text-muted-foreground">Keep all data on your device</div>
                    </div>
                    <Button variant="outline" disabled>
                      On
                    </Button>
                  </div>
                </div>
              </div>
            )}

            {activeSection === "billing" && (
              <div className="space-y-4">
                <h3 className="text-lg font-semibold">Billing</h3>
                <div className="space-y-4">
                  <div className="p-4 border rounded-lg bg-muted/50">
                    <div className="font-semibold mb-2">PrometheOS Lite</div>
                    <div className="text-sm text-muted-foreground mb-4">
                      You're currently on the free Lite plan. Upgrade to Pro for advanced features.
                    </div>
                    <Button className="w-full" disabled>
                      Upgrade to Pro
                    </Button>
                    <div className="text-xs text-muted-foreground mt-2 text-center">
                      Pro features coming soon
                    </div>
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
