
import { useMemo, useState } from "react";
import { NavLink, useNavigate } from "react-router-dom";
import { Folder, Plus, MessageSquare, LogOut, User, PanelLeft, PanelRight, Brain, Code2, FileText, Lightbulb, Rocket, Star, ChevronDown, MoreHorizontal, Settings } from "lucide-react";
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarFooter,
  SidebarHeader,
  useSidebar,
} from "@/components/ui/sidebar";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger, DropdownMenuSeparator, DropdownMenuSub, DropdownMenuSubTrigger, DropdownMenuSubContent } from "@/components/ui/dropdown-menu";
import { useChat } from "@/store/chat";
import { useAuth } from "@/hooks/useAuth";
import Logo from "@/components/Logo";
import SettingsDialog from "@/components/settings/SettingsDialog";

const EditableText: React.FC<{
  value: string;
  onChange: (v: string) => void;
  className?: string;
}> = ({ value, onChange, className }) => {
  const [editing, setEditing] = useState(false);
  const [temp, setTemp] = useState(value);

  return editing ? (
    <input
      className={`w-full bg-transparent border-b outline-none focus:border-primary ${className || ""}`}
      value={temp}
      onChange={(e) => setTemp(e.target.value)}
      onClick={(e) => e.stopPropagation()}
      onBlur={() => {
        setEditing(false);
        if (temp.trim() && temp !== value) onChange(temp.trim());
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        if (e.key === "Escape") {
          setTemp(value);
          setEditing(false);
        }
      }}
      autoFocus
    />
  ) : (
    <button 
      className={`truncate text-left ${className || ""}`} 
      onClick={(e) => { e.stopPropagation(); setEditing(true); }}
    >
      {value}
    </button>
  );
};

export default function ChatSidebar() {
  const { state, toggleSidebar } = useSidebar();
  const collapsed = state === 'collapsed';
  const {
    projects,
    conversations,
    currentConversationId,
    createConversation,
    setCurrentConversation,
    createProject,
    renameProject,
    setProjectIcon,
    deleteProject,
    renameConversation,
    deleteConversation,
    moveConversationToProject,
    setConversationIcon,
  } = useChat();

  const iconMap = useMemo(() => ({
    "message-square": MessageSquare,
    brain: Brain,
    "code-2": Code2,
    "file-text": FileText,
    lightbulb: Lightbulb,
    rocket: Rocket,
    star: Star,
  }), []);

  const iconOptions = [
    { key: "message-square", label: "Chat", Icon: MessageSquare },
    { key: "brain", label: "Brainstorm", Icon: Brain },
    { key: "code-2", label: "Code", Icon: Code2 },
    { key: "file-text", label: "Docs", Icon: FileText },
    { key: "lightbulb", label: "Idea", Icon: Lightbulb },
    { key: "rocket", label: "Launch", Icon: Rocket },
    { key: "star", label: "Starred", Icon: Star },
  ] as const;

  const projectIconOptions = [
    { key: "folder", label: "Folder", Icon: Folder },
    { key: "lightbulb", label: "Idea", Icon: Lightbulb },
    { key: "rocket", label: "Launch", Icon: Rocket },
    { key: "star", label: "Starred", Icon: Star },
    { key: "code-2", label: "Code", Icon: Code2 },
    { key: "brain", label: "Brainstorm", Icon: Brain },
  ] as const;

  const RenderIcon = ({ name, className }: { name: string; className?: string }) => {
    const C = (iconMap as any)[name] || MessageSquare;
    return <C className={className} />;
  };

  const grouped = useMemo(() => {
    const map: Record<string, { projectName: string; items: typeof conversations }> = {} as any;
    for (const p of projects) {
      (map as any)[p.id] = { projectName: p.name, items: [] } as any;
    }
    for (const c of conversations) {
      const pid = c.projectId || "unsorted";
      if (!(map as any)[pid]) (map as any)[pid] = { projectName: "Unsorted", items: [] } as any;
      (map as any)[pid].items.push(c as any);
    }
    return map as any;
  }, [projects, conversations]);

  const projectList = useMemo(() => projects.filter((p) => p.id !== "unsorted"), [projects]);

  return (
    <Sidebar collapsible="icon" className="group-data-[collapsible=icon]:border-0">
      <SidebarContent 
        className="group-data-[collapsible=icon]:overflow-visible" 
        onClick={(e) => {
          // Only toggle if clicking on free space (not on interactive elements)
          if (e.target === e.currentTarget) {
            toggleSidebar();
          }
        }}
      >
        <SidebarHeader>
          <div className="flex items-center justify-between">
            <button onClick={toggleSidebar} className="cursor-pointer">
              <Logo size="h-6 w-6" />
            </button>
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  className="w-8 h-8 rounded-md flex items-center justify-center text-muted-foreground hover:bg-muted"
                  onClick={(e) => { e.stopPropagation(); toggleSidebar(); }}
                  aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
                >
                  {collapsed ? <PanelRight className="h-4 w-4" /> : <PanelLeft className="h-4 w-4" />}
                </button>
              </TooltipTrigger>
              <TooltipContent side="right">{collapsed ? "Expand" : "Collapse"}</TooltipContent>
            </Tooltip>
          </div>

          <div className="mt-4">
            {collapsed ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    className="w-10 h-10 rounded-md flex items-center justify-center hover:bg-muted"
                    onClick={(e) => { e.stopPropagation(); createConversation(); }}
                    aria-label="New chat"
                  >
                    <Plus className="h-4 w-4" />
                  </button>
                </TooltipTrigger>
                <TooltipContent side="right">New chat</TooltipContent>
              </Tooltip>
            ) : (
              <button
                className="w-full h-10 inline-flex items-center justify-center gap-2 px-3 py-2 rounded-md border hover:bg-muted transition-colors"
                onClick={(e) => { e.stopPropagation(); createConversation(); }}
              >
                <Plus className="h-4 w-4" />
                <span>New chat</span>
              </button>
            )}
          </div>
        </SidebarHeader>

        {/* Projects section */}
        <SidebarGroup>
          <SidebarGroupLabel className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <Folder className="h-4 w-4" />
              <span className="font-medium">Projects</span>
            </div>
            {!collapsed && (
              <button
                className="text-muted-foreground hover:text-foreground"
                onClick={() => createProject()}
                aria-label="New project"
              >
                <Plus className="h-4 w-4" />
              </button>
            )}
          </SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {projectList.map((p) => (
                <SidebarMenuItem key={p.id}>
                  {collapsed ? (
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <SidebarMenuButton asChild>
                          <button className="flex items-center justify-center w-10 h-10" aria-label={p.name}>
                            <RenderIcon name={p.icon} className="h-4 w-4" />
                          </button>
                        </SidebarMenuButton>
                      </TooltipTrigger>
                      <TooltipContent side="right">{p.name}</TooltipContent>
                    </Tooltip>
                  ) : (
                    <div className="rounded-md">
                      <div className="flex items-center justify-between gap-2 px-2 py-2 rounded-md hover:bg-muted/50">
                        <div className="flex items-center gap-2 min-w-0">
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild>
                              <button
                                className="w-6 h-6 rounded-md hover:bg-muted/70 flex items-center justify-center flex-shrink-0"
                                onClick={(e) => { e.preventDefault(); e.stopPropagation(); }}
                                aria-label="Choose project icon"
                              >
                                <RenderIcon name={p.icon} className="h-4 w-4" />
                              </button>
                            </DropdownMenuTrigger>
                            <DropdownMenuContent className="z-50 bg-popover" align="start" side="right">
                              {projectIconOptions.map((opt) => (
                                <DropdownMenuItem
                                  key={opt.key}
                                  onClick={(e) => { e.preventDefault(); setProjectIcon(p.id, opt.key); }}
                                >
                                  <opt.Icon className="h-4 w-4 mr-2" /> {opt.label}
                                </DropdownMenuItem>
                              ))}
                            </DropdownMenuContent>
                          </DropdownMenu>
                          <EditableText
                            value={p.name}
                            onChange={(v) => renameProject(p.id, v)}
                            className="truncate flex-1"
                          />
                        </div>
                        <div className="opacity-0 hover:opacity-100 focus-within:opacity-100 group-hover:opacity-100 transition-opacity flex-shrink-0">
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild>
                              <button
                                className="w-6 h-6 rounded-md hover:bg-muted/70 flex items-center justify-center"
                                onClick={(e) => { e.preventDefault(); e.stopPropagation(); }}
                                aria-label="More actions"
                              >
                                <MoreHorizontal className="h-4 w-4" />
                              </button>
                            </DropdownMenuTrigger>
                            <DropdownMenuContent align="end" className="z-50 bg-popover">
                              <DropdownMenuItem onClick={(e) => { e.preventDefault(); const t = prompt('Rename project', p.name); if (t && t.trim()) renameProject(p.id, t.trim()); }}>Edit name</DropdownMenuItem>
                              <DropdownMenuSeparator />
                              <DropdownMenuItem className="text-destructive" onClick={(e) => { e.preventDefault(); if (confirm('Delete this project? Chats will move to Unsorted.')) deleteProject(p.id); }}>Delete project</DropdownMenuItem>
                            </DropdownMenuContent>
                          </DropdownMenu>
                        </div>
                      </div>

                      {grouped[p.id]?.items?.length ? (
                        <div className="pl-8 pb-1">
                          {grouped[p.id].items.map((c) => (
                            <div
                              key={c.id}
                              className={`flex items-center justify-between gap-2 px-2 py-2 rounded-md hover:bg-muted/50 ${currentConversationId === c.id ? "bg-muted" : ""}`}
                              onClick={() => setCurrentConversation(c.id)}
                            >
                              <div className="flex items-center gap-2 min-w-0">
                                <DropdownMenu>
                                  <DropdownMenuTrigger asChild>
                                    <button
                                      className="w-6 h-6 rounded-md hover:bg-muted/70 flex items-center justify-center flex-shrink-0"
                                      onClick={(e) => { e.preventDefault(); e.stopPropagation(); }}
                                      aria-label="Choose chat icon"
                                    >
                                      <RenderIcon name={c.icon} className="h-4 w-4" />
                                    </button>
                                  </DropdownMenuTrigger>
                                  <DropdownMenuContent className="z-50 bg-popover" align="start" side="right">
                                    {iconOptions.map((opt) => (
                                      <DropdownMenuItem
                                        key={opt.key}
                                        onClick={(e) => { e.preventDefault(); setConversationIcon(c.id, opt.key); }}
                                      >
                                        <opt.Icon className="h-4 w-4 mr-2" /> {opt.label}
                                      </DropdownMenuItem>
                                    ))}
                                  </DropdownMenuContent>
                                </DropdownMenu>
                                <EditableText
                                  value={c.title}
                                  onChange={(v) => renameConversation(c.id, v)}
                                  className="truncate flex-1"
                                />
                              </div>
                              <div className="opacity-0 hover:opacity-100 focus-within:opacity-100 group-hover:opacity-100 transition-opacity flex-shrink-0">
                                <DropdownMenu>
                                  <DropdownMenuTrigger asChild>
                                    <button
                                      className="w-6 h-6 rounded-md hover:bg-muted/70 flex items-center justify-center"
                                      onClick={(e) => { e.preventDefault(); e.stopPropagation(); }}
                                      aria-label="More actions"
                                    >
                                      <MoreHorizontal className="h-4 w-4" />
                                    </button>
                                  </DropdownMenuTrigger>
                                   <DropdownMenuContent align="end" className="z-50 bg-popover">
                                     <DropdownMenuItem onClick={(e) => { e.preventDefault(); const t = prompt('Rename chat', c.title); if (t && t.trim()) renameConversation(c.id, t.trim()); }}>Edit name</DropdownMenuItem>
                                     <DropdownMenuSub>
                                       <DropdownMenuSubTrigger>Change icon</DropdownMenuSubTrigger>
                                       <DropdownMenuSubContent>
                                         {iconOptions.map((opt) => (
                                           <DropdownMenuItem key={opt.key} onClick={(e) => { e.preventDefault(); setConversationIcon(c.id, opt.key); }}>
                                             <div className="flex items-center gap-2">
                                               <opt.Icon className="h-4 w-4" />
                                               <span>{opt.label}</span>
                                             </div>
                                           </DropdownMenuItem>
                                         ))}
                                       </DropdownMenuSubContent>
                                     </DropdownMenuSub>
                                     <DropdownMenuSub>
                                       <DropdownMenuSubTrigger>Move to project</DropdownMenuSubTrigger>
                                      <DropdownMenuSubContent>
                                        <DropdownMenuItem onClick={(e) => { e.preventDefault(); moveConversationToProject(c.id, 'unsorted'); }}>Unsorted</DropdownMenuItem>
                                        {projectList.map((p2) => (
                                          <DropdownMenuItem key={p2.id} onClick={(e) => { e.preventDefault(); moveConversationToProject(c.id, p2.id); }}>
                                            {p2.name}
                                          </DropdownMenuItem>
                                        ))}
                                      </DropdownMenuSubContent>
                                    </DropdownMenuSub>
                                    <DropdownMenuSeparator />
                                    <DropdownMenuItem className="text-destructive" onClick={(e) => { e.preventDefault(); if (confirm('Delete this chat?')) deleteConversation(c.id); }}>Delete chat</DropdownMenuItem>
                                  </DropdownMenuContent>
                                </DropdownMenu>
                              </div>
                            </div>
                          ))}
                        </div>
                      ) : null}
                    </div>
                  )}
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>

        {/* Chats section */}
        <SidebarGroup>
          <SidebarGroupLabel className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <MessageSquare className="h-4 w-4" />
              <span className="font-medium">Chats</span>
            </div>
            {!collapsed && (
              <button
                className="text-muted-foreground hover:text-foreground"
                onClick={() => createConversation()}
                aria-label="New chat"
              >
                <Plus className="h-4 w-4" />
              </button>
            )}
          </SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {conversations.filter((c) => !c.projectId || c.projectId === 'unsorted').map((c) => (
                <SidebarMenuItem key={c.id}>
                  {collapsed ? (
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <SidebarMenuButton asChild>
                          <button
                            className="flex items-center justify-center w-10 h-10"
                            onClick={() => setCurrentConversation(c.id)}
                            aria-label={c.title}
                          >
                            <RenderIcon name={c.icon} className="h-4 w-4" />
                          </button>
                        </SidebarMenuButton>
                      </TooltipTrigger>
                      <TooltipContent side="right">{c.title}</TooltipContent>
                    </Tooltip>
                  ) : (
                    <div
                      className={`flex items-center justify-between gap-2 px-2 py-2 rounded-md hover:bg-muted/50 ${
                        currentConversationId === c.id ? "bg-muted" : ""
                      }`}
                      onClick={() => setCurrentConversation(c.id)}
                    >
                      <div className="flex items-center gap-2 min-w-0">
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <button
                              className="w-6 h-6 rounded-md hover:bg-muted/70 flex items-center justify-center flex-shrink-0"
                              onClick={(e) => { e.preventDefault(); e.stopPropagation(); }}
                              aria-label="Choose chat icon"
                            >
                              <RenderIcon name={c.icon} className="h-4 w-4" />
                            </button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent className="z-50 bg-popover" align="start" side="right">
                            {iconOptions.map((opt) => (
                              <DropdownMenuItem
                                key={opt.key}
                                onClick={(e) => { e.preventDefault(); setConversationIcon(c.id, opt.key); }}
                              >
                                <opt.Icon className="h-4 w-4 mr-2" /> {opt.label}
                              </DropdownMenuItem>
                            ))}
                          </DropdownMenuContent>
                        </DropdownMenu>
                        <EditableText
                          value={c.title}
                          onChange={(v) => renameConversation(c.id, v)}
                          className="truncate flex-1"
                        />
                      </div>
                      <div className="opacity-0 hover:opacity-100 focus-within:opacity-100 group-hover:opacity-100 transition-opacity flex-shrink-0">
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <button
                              className="w-6 h-6 rounded-md hover:bg-muted/70 flex items-center justify-center"
                              onClick={(e) => { e.preventDefault(); e.stopPropagation(); }}
                              aria-label="More actions"
                            >
                              <MoreHorizontal className="h-4 w-4" />
                            </button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end" className="z-50 bg-popover">
                            <DropdownMenuItem onClick={(e) => { e.preventDefault(); const t = prompt('Rename chat', c.title); if (t && t.trim()) renameConversation(c.id, t.trim()); }}>Edit name</DropdownMenuItem>
                            <DropdownMenuSub>
                              <DropdownMenuSubTrigger>Move to project</DropdownMenuSubTrigger>
                              <DropdownMenuSubContent>
                                <DropdownMenuItem onClick={(e) => { e.preventDefault(); moveConversationToProject(c.id, 'unsorted'); }}>Unsorted</DropdownMenuItem>
                                {projectList.map((p) => (
                                  <DropdownMenuItem key={p.id} onClick={(e) => { e.preventDefault(); moveConversationToProject(c.id, p.id); }}>
                                    {p.name}
                                  </DropdownMenuItem>
                                ))}
                              </DropdownMenuSubContent>
                            </DropdownMenuSub>
                            <DropdownMenuSeparator />
                            <DropdownMenuItem className="text-destructive" onClick={(e) => { e.preventDefault(); if (confirm('Delete this chat?')) deleteConversation(c.id); }}>Delete chat</DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </div>
                    </div>
                  )}
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>

        <SidebarFooter className="mt-auto">
          <UserArea collapsed={collapsed} />
        </SidebarFooter>
      </SidebarContent>
    </Sidebar>
  );
}

function UserArea({ collapsed }: { collapsed: boolean }) {
  const { user, signOut } = useAuth();
  const navigate = useNavigate();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const avatarUrl = (user?.user_metadata as any)?.avatar_url as string | undefined;
  const firstName = (user?.user_metadata as any)?.first_name as string | undefined;
  const lastName = (user?.user_metadata as any)?.last_name as string | undefined;
  const fullName = firstName && lastName ? `${firstName} ${lastName}` : firstName || user?.email?.split('@')[0] || "User";
  const initials = fullName.slice(0, 2).toUpperCase();

  if (!user) {
    return collapsed ? (
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            className="w-10 h-10 rounded-full flex items-center justify-center hover:bg-muted"
            onClick={() => navigate("/auth")}
            aria-label="Sign in"
          >
            <User className="h-5 w-5" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="right">Sign in</TooltipContent>
      </Tooltip>
    ) : (
      <button
        onClick={() => navigate("/auth")}
        className="w-full h-10 inline-flex items-center justify-center gap-2 px-3 py-2 rounded-md border hover:bg-muted"
      >
        <User className="h-4 w-4" /> <span>Sign in</span>
      </button>
    );
  }

  return collapsed ? (
    <>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button className="w-10 h-10 rounded-full overflow-hidden" aria-label="Account">
                <Avatar className="w-10 h-10">
                  <AvatarImage src={avatarUrl} alt="User avatar" />
                  <AvatarFallback>{initials}</AvatarFallback>
                </Avatar>
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent side="right" align="start">
              <DropdownMenuItem onClick={() => setSettingsOpen(true)}>
                <Settings className="h-4 w-4 mr-2" /> Settings
              </DropdownMenuItem>
              <DropdownMenuItem onClick={signOut} className="text-destructive">
                <LogOut className="h-4 w-4 mr-2" /> Logout
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </TooltipTrigger>
        <TooltipContent side="right">{fullName}</TooltipContent>
      </Tooltip>
      <SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
    </>
  ) : (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button className="w-full h-10 inline-flex items-center justify-start gap-3 px-3 py-2 rounded-md border hover:bg-muted">
            <Avatar className="w-6 h-6 flex-shrink-0">
              <AvatarImage src={avatarUrl} alt="User avatar" />
              <AvatarFallback>{initials}</AvatarFallback>
            </Avatar>
            <span className="truncate text-left">{fullName}</span>
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuItem onClick={() => setSettingsOpen(true)}>
            <Settings className="h-4 w-4 mr-2" /> Settings
          </DropdownMenuItem>
          <DropdownMenuItem onClick={signOut} className="text-destructive">
            <LogOut className="h-4 w-4 mr-2" /> Logout
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
    </>
  );
}
