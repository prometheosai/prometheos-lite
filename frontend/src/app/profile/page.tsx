"use client"

import { useState } from "react"
import { User, Mail, Calendar, MessageSquare, Folder } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar"
import { Separator } from "@/components/ui/separator"
import { AppLayout } from "@/components/layout/app-layout"
import { useChat } from "@/context/chat-context"

export default function ProfilePage() {
  const { projects, conversations } = useChat()
  const [name, setName] = useState("User")
  const [email, setEmail] = useState("")

  const stats = {
    projects: projects.filter(p => p.id !== "unsorted").length,
    conversations: conversations.length,
    messages: conversations.reduce((acc, c) => acc + c.messages.length, 0),
  }

  return (
    <AppLayout hideConversation>
      <div className="h-16 border-b border-border flex items-center px-6">
        <h1 className="text-xl font-display font-semibold text-foreground">Profile</h1>
      </div>
      
      <div className="flex-1 p-6 max-w-4xl mx-auto w-full overflow-y-auto">
        <div className="space-y-8">
          {/* Profile Header */}
          <div className="prometheos-surface p-6">
            <div className="flex items-start gap-6">
              <Avatar className="h-24 w-24">
                <AvatarImage src="" />
                <AvatarFallback className="text-2xl">
                  {name.slice(0, 2).toUpperCase()}
                </AvatarFallback>
              </Avatar>
              <div className="flex-1 space-y-4">
                <div>
                  <h2 className="text-2xl font-display font-semibold text-foreground">
                    {name}
                  </h2>
                  <p className="text-muted-foreground">PrometheOS User</p>
                </div>
                <div className="flex gap-2">
                  <Button>Edit Profile</Button>
                  <Button variant="outline">Change Avatar</Button>
                </div>
              </div>
            </div>
          </div>

          <Separator />

          {/* Profile Form */}
          <div className="space-y-4">
            <h3 className="text-lg font-semibold text-foreground">Profile Information</h3>
            <div className="space-y-4 prometheos-surface p-6">
              <div className="space-y-2">
                <Label htmlFor="name">Display Name</Label>
                <Input
                  id="name"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  className="bg-background"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="email">Email (Optional)</Label>
                <Input
                  id="email"
                  type="email"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  placeholder="your@email.com"
                  className="bg-background"
                />
              </div>
              <Button>Save Changes</Button>
            </div>
          </div>

          <Separator />

          {/* Statistics */}
          <div className="space-y-4">
            <h3 className="text-lg font-semibold text-foreground">Statistics</h3>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              <div className="prometheos-surface p-6">
                <div className="flex items-center gap-3">
                  <div className="p-3 rounded-lg bg-primary/10">
                    <Folder className="h-6 w-6 text-primary" />
                  </div>
                  <div>
                    <p className="text-2xl font-display font-semibold text-foreground">
                      {stats.projects}
                    </p>
                    <p className="text-sm text-muted-foreground">Projects</p>
                  </div>
                </div>
              </div>
              <div className="prometheos-surface p-6">
                <div className="flex items-center gap-3">
                  <div className="p-3 rounded-lg bg-primary/10">
                    <MessageSquare className="h-6 w-6 text-primary" />
                  </div>
                  <div>
                    <p className="text-2xl font-display font-semibold text-foreground">
                      {stats.conversations}
                    </p>
                    <p className="text-sm text-muted-foreground">Conversations</p>
                  </div>
                </div>
              </div>
              <div className="prometheos-surface p-6">
                <div className="flex items-center gap-3">
                  <div className="p-3 rounded-lg bg-primary/10">
                    <Mail className="h-6 w-6 text-primary" />
                  </div>
                  <div>
                    <p className="text-2xl font-display font-semibold text-foreground">
                      {stats.messages}
                    </p>
                    <p className="text-sm text-muted-foreground">Messages</p>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <Separator />

          {/* Account Info */}
          <div className="space-y-4">
            <h3 className="text-lg font-semibold text-foreground">Account Information</h3>
            <div className="space-y-4 prometheos-surface p-6">
              <div className="flex items-center gap-3 text-sm">
                <Calendar className="h-4 w-4 text-muted-foreground" />
                <span className="text-muted-foreground">Member since:</span>
                <span className="text-foreground">Today</span>
              </div>
              <div className="flex items-center gap-3 text-sm">
                <User className="h-4 w-4 text-muted-foreground" />
                <span className="text-muted-foreground">Account type:</span>
                <span className="text-foreground">Local (Unauthenticated)</span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </AppLayout>
  )
}
