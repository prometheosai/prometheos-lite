"use client"

import { useState } from "react"
import { Settings, Key, Palette, Database, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Separator } from "@/components/ui/separator"
import { useTheme } from "next-themes"
import { AppLayout } from "@/components/layout/app-layout"

export default function SettingsPage() {
  const { theme, setTheme } = useTheme()
  const [apiKey, setApiKey] = useState("")
  const [modelName, setModelName] = useState("")

  const handleClearData = () => {
    if (confirm("Are you sure you want to clear all local data? This cannot be undone.")) {
      localStorage.clear()
      window.location.reload()
    }
  }

  return (
    <AppLayout hideConversation>
      <div className="h-16 border-b border-border flex items-center px-6">
        <h1 className="text-xl font-display font-semibold text-foreground">Settings</h1>
      </div>
      
      <div className="flex-1 p-6 max-w-4xl mx-auto w-full overflow-y-auto">
        <div className="space-y-8">
          {/* API Configuration */}
          <div className="space-y-4">
            <div className="flex items-center gap-2">
              <Key className="h-5 w-5 text-foreground" />
              <h2 className="text-lg font-semibold text-foreground">API Configuration</h2>
            </div>
            <div className="space-y-4 prometheos-surface p-6">
              <div className="space-y-2">
                <Label htmlFor="api-key">OpenAI API Key</Label>
                <Input
                  id="api-key"
                  type="password"
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
                  placeholder="sk-..."
                  className="bg-background"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="model-name">Model Name</Label>
                <Input
                  id="model-name"
                  value={modelName}
                  onChange={(e) => setModelName(e.target.value)}
                  placeholder="gpt-4"
                  className="bg-background"
                />
              </div>
              <Button>Save API Configuration</Button>
            </div>
          </div>

          <Separator />

          {/* Appearance */}
          <div className="space-y-4">
            <div className="flex items-center gap-2">
              <Palette className="h-5 w-5 text-foreground" />
              <h2 className="text-lg font-semibold text-foreground">Appearance</h2>
            </div>
            <div className="space-y-4 prometheos-surface p-6">
              <div className="space-y-2">
                <Label>Theme</Label>
                <div className="flex gap-2">
                  <Button
                    variant={theme === "dark" ? "default" : "outline"}
                    onClick={() => setTheme("dark")}
                  >
                    Dark
                  </Button>
                  <Button
                    variant={theme === "light" ? "default" : "outline"}
                    onClick={() => setTheme("light")}
                  >
                    Light
                  </Button>
                  <Button
                    variant={theme === "system" ? "default" : "outline"}
                    onClick={() => setTheme("system")}
                  >
                    System
                  </Button>
                </div>
              </div>
            </div>
          </div>

          <Separator />

          {/* Data Management */}
          <div className="space-y-4">
            <div className="flex items-center gap-2">
              <Database className="h-5 w-5 text-foreground" />
              <h2 className="text-lg font-semibold text-foreground">Data Management</h2>
            </div>
            <div className="space-y-4 prometheos-surface p-6">
              <div className="flex items-center justify-between">
                <div>
                  <p className="font-medium text-foreground">Clear Local Data</p>
                  <p className="text-sm text-muted-foreground">
                    Remove all locally stored projects, conversations, and settings
                  </p>
                </div>
                <Button variant="destructive" onClick={handleClearData}>
                  <Trash2 className="h-4 w-4 mr-2" />
                  Clear Data
                </Button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </AppLayout>
  )
}
