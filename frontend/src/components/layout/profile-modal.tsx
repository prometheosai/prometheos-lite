"use client"

import { useState, useRef } from "react"
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import { Avatar, AvatarFallback } from "@/components/ui/avatar"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { useTheme } from "next-themes"
import { LogOut, Settings, User as UserIcon, Cpu, Brain, Shield, CreditCard, Moon, Sun, AlertTriangle, Trash2, Key, RotateCcw, MessageSquare, Zap, Lock } from "lucide-react"
import { useProfile } from "@/context/profile-context"
import ReactCrop, { Crop, PixelCrop } from 'react-image-crop'
import 'react-image-crop/dist/ReactCrop.css'

interface ProfileModalProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function ProfileModal({ open, onOpenChange }: ProfileModalProps) {
  const { theme, setTheme } = useTheme()
  const { profile, updateProfile } = useProfile()
  const [activeSection, setActiveSection] = useState<"account" | "preferences" | "models" | "memory" | "privacy" | "billing" | "security" | "channels" | "mcp">("account")
  const [isEditingName, setIsEditingName] = useState(false)
  const [isEditingEmail, setIsEditingEmail] = useState(false)
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false)
  const [showCropModal, setShowCropModal] = useState(false)
  
  const [tempProfile, setTempProfile] = useState(profile)
  const [password, setPassword] = useState("")
  const [newPassword, setNewPassword] = useState("")
  const [confirmPassword, setConfirmPassword] = useState("")
  const fileInputRef = useRef<HTMLInputElement>(null)
  
  const [selectedImage, setSelectedImage] = useState<string | null>(null)
  const [crop, setCrop] = useState<Crop>({
    unit: '%',
    width: 100,
    height: 100,
    x: 0,
    y: 0
  })
  const [completedCrop, setCompletedCrop] = useState<PixelCrop | null>(null)
  const imageRef = useRef<HTMLImageElement>(null)

  const handleImageUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (file) {
      const reader = new FileReader()
      reader.onloadend = () => {
        setSelectedImage(reader.result as string)
        setShowCropModal(true)
      }
      reader.readAsDataURL(file)
    }
  }

  const handleCropComplete = (crop: PixelCrop) => {
    setCompletedCrop(crop)
  }

  const handleApplyCrop = () => {
    if (completedCrop && imageRef.current && selectedImage) {
      const canvas = document.createElement('canvas')
      const ctx = canvas.getContext('2d')
      
      if (!ctx) return

      const scaleX = imageRef.current.naturalWidth / imageRef.current.width
      const scaleY = imageRef.current.naturalHeight / imageRef.current.height

      canvas.width = 200
      canvas.height = 200

      ctx.drawImage(
        imageRef.current,
        completedCrop.x * scaleX,
        completedCrop.y * scaleY,
        completedCrop.width * scaleX,
        completedCrop.height * scaleY,
        0,
        0,
        canvas.width,
        canvas.height
      )

      const croppedImageUrl = canvas.toDataURL('image/jpeg')
      updateProfile({ ...profile, avatarUrl: croppedImageUrl })
      setShowCropModal(false)
      setSelectedImage(null)
      setCrop({
        unit: '%',
        width: 100,
        height: 100,
        x: 0,
        y: 0
      })
      setCompletedCrop(null)
    }
  }

  const handleCancelCrop = () => {
    setShowCropModal(false)
    setSelectedImage(null)
    setCrop({
      unit: '%',
      width: 100,
      height: 100,
      x: 0,
      y: 0
    })
    setCompletedCrop(null)
  }

  const handleRemoveAvatar = () => {
    updateProfile({ ...profile, avatarUrl: null })
  }

  const sections = [
    { id: "account", label: "Account", icon: UserIcon },
    { id: "billing", label: "Billing", icon: CreditCard },
    { id: "channels", label: "Channels", icon: MessageSquare },
    { id: "memory", label: "Memory", icon: Brain },
    { id: "mcp", label: "MCP", icon: Zap },
    { id: "models", label: "Models", icon: Cpu },
    { id: "preferences", label: "Preferences", icon: Settings },
    { id: "privacy", label: "Privacy", icon: Shield },
    { id: "security", label: "Security", icon: Lock },
  ] as const

  const handleSaveProfile = () => {
    updateProfile(tempProfile)
    setIsEditingName(false)
    setIsEditingEmail(false)
  }

  const handleCancelEdit = () => {
    setTempProfile(profile)
    setIsEditingName(false)
    setIsEditingEmail(false)
  }

  const handleDeleteAccount = () => {
    // TODO: Implement actual account deletion API call
    console.log("Deleting account and all data")
    localStorage.clear()
    setShowDeleteConfirm(false)
    onOpenChange(false)
  }

  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent className="max-w-2xl max-h-[80vh] overflow-hidden">
          <DialogHeader>
            <DialogTitle>Profile & Settings</DialogTitle>
          </DialogHeader>

          <div className="flex gap-6 mt-4 h-[60vh]">
            {/* Sidebar */}
            <div className="w-48 space-y-1 flex-shrink-0">
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
            <div className="flex-1 overflow-y-auto pr-2" style={{ scrollbarWidth: 'none', msOverflowStyle: 'none' }}>
              {activeSection === "account" && (
                <div className="space-y-4">
                  <h3 className="text-lg font-semibold">Account</h3>
                  <div className="space-y-4">
                    <div className="flex items-center gap-4">
                      <div className="relative cursor-pointer group">
                        <input
                          ref={fileInputRef}
                          type="file"
                          accept="image/*"
                          onChange={handleImageUpload}
                          className="hidden"
                        />
                        {profile.avatarUrl ? (
                          <Avatar className="h-16 w-16 cursor-pointer" onClick={() => fileInputRef.current?.click()}>
                            <img src={profile.avatarUrl} alt="Profile" className="h-full w-full object-cover" />
                          </Avatar>
                        ) : (
                          <Avatar className="h-16 w-16 cursor-pointer" onClick={() => fileInputRef.current?.click()}>
                            <AvatarFallback className="text-lg">{profile.initials}</AvatarFallback>
                          </Avatar>
                        )}
                        <div 
                          className="absolute inset-0 bg-black/50 rounded-full opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center pointer-events-none"
                        >
                          <span className="text-white text-xs">Change</span>
                        </div>
                      </div>
                      <div className="flex-1 space-y-2">
                        {profile.avatarUrl && (
                          <Button size="sm" variant="outline" onClick={handleRemoveAvatar}>Remove</Button>
                        )}
                        {isEditingName ? (
                          <div className="flex gap-2">
                            <Input
                              value={tempProfile.name}
                              onChange={(e) => setTempProfile({ ...tempProfile, name: e.target.value })}
                              className="flex-1"
                            />
                            <Button size="sm" onClick={handleSaveProfile}>Save</Button>
                            <Button size="sm" variant="outline" onClick={handleCancelEdit}>Cancel</Button>
                          </div>
                        ) : (
                          <div className="font-semibold text-lg cursor-pointer hover:text-primary transition-colors" onClick={() => { setIsEditingName(true); setTempProfile(profile) }}>
                            {profile.name}
                          </div>
                        )}
                        {isEditingEmail ? (
                          <div className="flex gap-2">
                            <Input
                              value={tempProfile.email}
                              onChange={(e) => setTempProfile({ ...tempProfile, email: e.target.value })}
                              className="flex-1"
                            />
                            <Button size="sm" onClick={handleSaveProfile}>Save</Button>
                            <Button size="sm" variant="outline" onClick={handleCancelEdit}>Cancel</Button>
                          </div>
                        ) : (
                          <div className="text-sm text-muted-foreground cursor-pointer hover:text-primary transition-colors" onClick={() => { setIsEditingEmail(true); setTempProfile(profile) }}>
                            {profile.email}
                          </div>
                        )}
                      </div>
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

            {activeSection === "security" && (
              <div className="space-y-4">
                <h3 className="text-lg font-semibold">Security</h3>
                <div className="space-y-4">
                  <div className="space-y-2">
                    <Label>Current Password</Label>
                    <Input
                      type="password"
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      placeholder="Enter current password"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label>New Password</Label>
                    <Input
                      type="password"
                      value={newPassword}
                      onChange={(e) => setNewPassword(e.target.value)}
                      placeholder="Enter new password"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label>Confirm New Password</Label>
                    <Input
                      type="password"
                      value={confirmPassword}
                      onChange={(e) => setConfirmPassword(e.target.value)}
                      placeholder="Confirm new password"
                    />
                  </div>
                  <Button
                    onClick={() => {
                      // TODO: Implement password change API call
                      console.log("Changing password")
                      setPassword("")
                      setNewPassword("")
                      setConfirmPassword("")
                    }}
                    disabled={!password || !newPassword || newPassword !== confirmPassword}
                  >
                    <Key className="h-4 w-4 mr-2" />
                    Change Password
                  </Button>

                  <Separator />

                  <div className="space-y-2">
                    <Button variant="outline" className="w-full justify-start">
                      <RotateCcw className="h-4 w-4 mr-2" />
                      Reset All Settings
                    </Button>
                  </div>

                  <Separator />

                  <div className="space-y-2">
                    {!showDeleteConfirm ? (
                      <Button
                        variant="destructive"
                        className="w-full justify-start"
                        onClick={() => setShowDeleteConfirm(true)}
                      >
                        <Trash2 className="h-4 w-4 mr-2" />
                        Delete Account
                      </Button>
                    ) : (
                      <Alert variant="destructive">
                        <AlertTriangle className="h-4 w-4" />
                        <AlertTitle>Delete Account</AlertTitle>
                        <AlertDescription className="space-y-4">
                          <div>
                            This action cannot be undone. This will permanently delete your account and remove all your data including projects, conversations, and memories.
                          </div>
                          <div className="flex gap-2">
                            <Button
                              variant="destructive"
                              size="sm"
                              onClick={handleDeleteAccount}
                            >
                              Confirm Delete
                            </Button>
                            <Button
                              variant="outline"
                              size="sm"
                              onClick={() => setShowDeleteConfirm(false)}
                            >
                              Cancel
                            </Button>
                          </div>
                        </AlertDescription>
                      </Alert>
                    )}
                  </div>
                </div>
              </div>
            )}

            {activeSection === "channels" && (
              <div className="space-y-4">
                <h3 className="text-lg font-semibold">Channels</h3>
                <p className="text-sm text-muted-foreground">Configure messaging channels to connect with your AI assistant</p>
                
                <div className="space-y-4">
                  {/* Telegram */}
                  <div className="border rounded-lg p-4 space-y-3">
                    <div className="flex items-center gap-3">
                      <div className="h-8 w-8 rounded-full bg-blue-500 flex items-center justify-center">
                        <svg className="h-5 w-5 text-white" viewBox="0 0 24 24" fill="currentColor">
                          <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm4.64 6.8c-.15 1.58-.8 5.42-1.13 7.19-.14.75-.42 1-.68 1.03-.58.05-1.02-.38-1.58-.75-.88-.58-1.38-.94-2.23-1.5-.99-.65-.35-1.01.22-1.59.15-.15 2.71-2.48 2.76-2.69a.2.2 0 00-.05-.18c-.06-.05-.14-.03-.21-.02-.09.02-1.49.95-4.22 2.79-.4.27-.76.41-1.08.4-.36-.01-1.04-.2-1.55-.37-.63-.2-1.12-.31-1.08-.66.02-.18.27-.36.74-.55 2.92-1.27 4.86-2.11 5.83-2.51 2.78-1.16 3.35-1.36 3.73-1.36.08 0 .27.02.39.12.1.08.13.19.14.27-.01.06.01.24 0 .38z"/>
                        </svg>
                      </div>
                      <div className="flex-1">
                        <div className="font-medium">Telegram</div>
                        <div className="text-xs text-muted-foreground">Connect via Telegram Bot</div>
                      </div>
                      <Button variant="outline" size="sm">Connect</Button>
                    </div>
                  </div>

                  {/* WhatsApp */}
                  <div className="border rounded-lg p-4 space-y-3">
                    <div className="flex items-center gap-3">
                      <div className="h-8 w-8 rounded-full bg-green-500 flex items-center justify-center">
                        <svg className="h-5 w-5 text-white" viewBox="0 0 24 24" fill="currentColor">
                          <path d="M17.472 14.382c-.297-.149-1.758-.867-2.03-.967-.273-.099-.471-.148-.67.15-.197.297-.767.966-.94 1.164-.173.199-.347.223-.644.075-.297-.15-1.255-.463-2.39-1.475-.883-.788-1.48-1.761-1.653-2.059-.173-.297-.018-.458.13-.606.134-.133.298-.347.446-.52.149-.174.198-.298.298-.497.099-.198.05-.371-.025-.52-.075-.149-.669-1.612-.916-2.207-.242-.579-.487-.5-.669-.51-.173-.008-.371-.01-.57-.01-.198 0-.52.074-.792.372-.272.297-1.04 1.016-1.04 2.479 0 1.462 1.065 2.875 1.213 3.074.149.198 2.096 3.2 5.077 4.487.709.306 1.262.489 1.694.625.712.227 1.36.195 1.871.118.571-.085 1.758-.719 2.006-1.413.248-.694.248-1.289.173-1.413-.074-.124-.272-.198-.57-.347m-5.421 7.403h-.004a9.87 9.87 0 01-5.031-1.378l-.361-.214-3.741.982.998-3.648-.235-.374a9.86 9.86 0 01-1.51-5.26c.001-5.45 4.436-9.884 9.888-9.884 2.64 0 5.122 1.03 6.988 2.898a9.825 9.825 0 012.893 6.994c-.003 5.45-4.437 9.884-9.885 9.884m8.413-18.297A11.815 11.815 0 0012.05 0C5.495 0 .16 5.335.157 11.892c0 2.096.547 4.142 1.588 5.945L.057 24l6.305-1.654a11.882 11.882 0 005.683 1.448h.005c6.554 0 11.89-5.335 11.893-11.893a11.821 11.821 0 00-3.48-8.413z"/>
                        </svg>
                      </div>
                      <div className="flex-1">
                        <div className="font-medium">WhatsApp</div>
                        <div className="text-xs text-muted-foreground">Connect via WhatsApp Business API</div>
                      </div>
                      <Button variant="outline" size="sm">Connect</Button>
                    </div>
                  </div>

                  {/* Discord */}
                  <div className="border rounded-lg p-4 space-y-3">
                    <div className="flex items-center gap-3">
                      <div className="h-8 w-8 rounded-full bg-indigo-500 flex items-center justify-center">
                        <svg className="h-5 w-5 text-white" viewBox="0 0 24 24" fill="currentColor">
                          <path d="M20.317 4.3698a19.7913 19.7913 0 00-4.8851-1.5152.0741.0741 0 00-.0785.0371c-.211.3753-.4447.8648-.6083 1.2495-1.8447-.2762-3.68-.2762-5.4868 0-.1636-.3933-.4058-.8742-.6177-1.2495a.077.077 0 00-.0785-.037 19.7363 19.7363 0 00-4.8852 1.515.0699.0699 0 00-.0321.0277C.5334 9.0458-.319 13.5799.0992 18.0578a.0824.0824 0 00.0312.0561c2.0528 1.5076 4.0413 2.4228 5.9929 3.0294a.0777.0777 0 00.0842-.0276c.4616-.6304.8731-1.2952 1.226-1.9942a.076.076 0 00-.0416-.1057c-.6528-.2476-1.2743-.5495-1.8722-.8923a.077.077 0 01-.0076-.1277c.1258-.0943.2517-.1923.3718-.2914a.0743.0743 0 01.0776-.0105c3.9278 1.7933 8.18 1.7933 12.0614 0a.0739.0739 0 01.0785.0095c.1202.099.246.1981.3728.2924a.077.077 0 01-.0066.1276 12.2986 12.2986 0 01-1.873.8914.0766.0766 0 00-.0407.1067c.3604.698.7719 1.3628 1.225 1.9932a.076.076 0 00.0842.0286c1.961-.6067 3.9495-1.5219 6.0023-3.0294a.077.077 0 00.0313-.0552c.5004-5.177-.8382-9.6739-3.5485-13.6604a.061.061 0 00-.0312-.0286zM8.02 15.3312c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9555-2.4189 2.157-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.9555 2.4189-2.1569 2.4189zm7.9748 0c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9554-2.4189 2.1569-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.946 2.4189-2.1568 2.4189z"/>
                        </svg>
                      </div>
                      <div className="flex-1">
                        <div className="font-medium">Discord</div>
                        <div className="text-xs text-muted-foreground">Connect via Discord Bot</div>
                      </div>
                      <Button variant="outline" size="sm">Connect</Button>
                    </div>
                  </div>

                  {/* Slack */}
                  <div className="border rounded-lg p-4 space-y-3">
                    <div className="flex items-center gap-3">
                      <div className="h-8 w-8 rounded-full bg-purple-600 flex items-center justify-center">
                        <svg className="h-5 w-5 text-white" viewBox="0 0 24 24" fill="currentColor">
                          <path d="M5.042 15.165a2.528 2.528 0 0 1-2.52 2.523A2.528 2.528 0 0 1 0 15.165a2.527 2.527 0 0 1 2.522-2.52h2.52v2.52zM6.313 15.165a2.527 2.527 0 0 1 2.521-2.52 2.527 2.527 0 0 1 2.521 2.52v6.313A2.528 2.528 0 0 1 8.834 24a2.528 2.528 0 0 1-2.521-2.522v-6.313zM8.834 5.042a2.528 2.528 0 0 1-2.521-2.52A2.528 2.528 0 0 1 8.834 0a2.528 2.528 0 0 1 2.521 2.522v2.52H8.834zM8.834 6.313a2.528 2.528 0 0 1 2.521 2.521 2.528 2.528 0 0 1-2.521 2.521H2.522A2.528 2.528 0 0 1 0 8.834a2.528 2.528 0 0 1 2.522-2.521h6.312zM18.956 8.834a2.528 2.528 0 0 1 2.522-2.521A2.528 2.528 0 0 1 24 8.834a2.528 2.528 0 0 1-2.522 2.521h-2.522V8.834zM17.688 8.834a2.528 2.528 0 0 1-2.523 2.521 2.527 2.527 0 0 1-2.52-2.521V2.522A2.527 2.527 0 0 1 15.165 0a2.528 2.528 0 0 1 2.523 2.522v6.312zM15.165 18.956a2.528 2.528 0 0 1 2.523 2.522A2.528 2.528 0 0 1 15.165 24a2.527 2.527 0 0 1-2.52-2.522v-2.522h2.52zM15.165 17.688a2.527 2.527 0 0 1-2.52-2.523 2.526 2.526 0 0 1 2.52-2.52h6.313A2.527 2.527 0 0 1 24 15.165a2.528 2.528 0 0 1-2.522 2.523h-6.313z"/>
                        </svg>
                      </div>
                      <div className="flex-1">
                        <div className="font-medium">Slack</div>
                        <div className="text-xs text-muted-foreground">Connect via Slack App</div>
                      </div>
                      <Button variant="outline" size="sm">Connect</Button>
                    </div>
                  </div>
                </div>

                <div className="text-xs text-muted-foreground mt-4">
                  More channels coming soon...
                </div>
              </div>
            )}

            {activeSection === "mcp" && (
              <div className="space-y-4">
                <h3 className="text-lg font-semibold">MCP Servers</h3>
                <p className="text-sm text-muted-foreground">Manage Model Context Protocol (MCP) servers to extend AI capabilities</p>
                
                <div className="space-y-4">
                  <div className="border rounded-lg p-4 space-y-3">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-3">
                        <div className="h-8 w-8 rounded-full bg-orange-500 flex items-center justify-center">
                          <Zap className="h-4 w-4 text-white" />
                        </div>
                        <div>
                          <div className="font-medium">Filesystem MCP</div>
                          <div className="text-xs text-muted-foreground">Access local files and directories</div>
                        </div>
                      </div>
                      <Button variant="outline" size="sm">Configure</Button>
                    </div>
                  </div>

                  <div className="border rounded-lg p-4 space-y-3">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-3">
                        <div className="h-8 w-8 rounded-full bg-blue-500 flex items-center justify-center">
                          <Zap className="h-4 w-4 text-white" />
                        </div>
                        <div>
                          <div className="font-medium">Database MCP</div>
                          <div className="text-xs text-muted-foreground">Query and manage databases</div>
                        </div>
                      </div>
                      <Button variant="outline" size="sm">Configure</Button>
                    </div>
                  </div>

                  <div className="border rounded-lg p-4 space-y-3">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-3">
                        <div className="h-8 w-8 rounded-full bg-green-500 flex items-center justify-center">
                          <Zap className="h-4 w-4 text-white" />
                        </div>
                        <div>
                          <div className="font-medium">Web Search MCP</div>
                          <div className="text-xs text-muted-foreground">Search the web for information</div>
                        </div>
                      </div>
                      <Button variant="outline" size="sm">Configure</Button>
                    </div>
                  </div>

                  <div className="border rounded-lg p-4 space-y-3">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-3">
                        <div className="h-8 w-8 rounded-full bg-purple-500 flex items-center justify-center">
                          <Zap className="h-4 w-4 text-white" />
                        </div>
                        <div>
                          <div className="font-medium">Git MCP</div>
                          <div className="text-xs text-muted-foreground">Interact with Git repositories</div>
                        </div>
                      </div>
                      <Button variant="outline" size="sm">Configure</Button>
                    </div>
                  </div>

                  <Button variant="outline" className="w-full">
                    <Zap className="h-4 w-4 mr-2" />
                    Add Custom MCP Server
                  </Button>
                </div>

                <div className="text-xs text-muted-foreground mt-4">
                  MCP servers allow the AI to interact with external tools and services
                </div>
              </div>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>

    {/* Crop Modal */}
    <Dialog open={showCropModal} onOpenChange={setShowCropModal}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Crop Profile Image</DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          {selectedImage && (
            <div className="space-y-4">
              <div className="max-h-[400px] overflow-hidden rounded-lg border">
                <ReactCrop
                  crop={crop}
                  onChange={(c) => setCrop(c)}
                  onComplete={handleCropComplete}
                  aspect={1}
                >
                  <img
                    ref={imageRef}
                    alt="Crop preview"
                    src={selectedImage}
                    className="max-w-full"
                  />
                </ReactCrop>
              </div>
              <div className="flex gap-2 justify-end">
                <Button variant="outline" onClick={handleCancelCrop}>Cancel</Button>
                <Button onClick={handleApplyCrop}>Apply Crop</Button>
              </div>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  </>
  )
}
