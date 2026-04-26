"use client"

import React, { createContext, useContext, useReducer, useEffect, useCallback, useMemo } from "react"
import { getProjects, createProject as createProjectApi, getConversations, createConversation as createConversationApi, getMessages, type Project, type Conversation, type Message } from "@/lib/api"

export type Role = "user" | "assistant" | "system"

export type ChatMessage = {
  id: string
  role: Role
  content: string
  createdAt: number
}

export type ChatConversation = {
  id: string
  title: string
  icon: string
  projectId: string | null
  messages: ChatMessage[]
  createdAt: number
  updatedAt: number
}

export type ChatProject = {
  id: string
  name: string
  icon: string
  createdAt: number
}

type State = {
  projects: ChatProject[]
  conversations: ChatConversation[]
  currentConversationId: string | null
  currentProjectId: string | null
  loading: boolean
  error: string | null
}

type ChatContextType = State & {
  currentConversation: ChatConversation | null
  createProject: (opts?: { name?: string; icon?: string }) => Promise<void>
  renameProject: (id: string, name: string) => void
  setProjectIcon: (id: string, icon: string) => void
  deleteProject: (id: string) => void
  archiveProject: (id: string) => void
  duplicateProject: (id: string) => void
  createConversation: (opts?: { title?: string; projectId?: string | null; icon?: string }) => Promise<string>
  renameConversation: (id: string, title: string) => void
  setConversationIcon: (id: string, icon: string) => void
  deleteConversation: (id: string) => void
  archiveConversation: (id: string) => void
  duplicateConversation: (id: string) => void
  setCurrentConversation: (id: string | null) => void
  setCurrentProject: (id: string | null) => void
  addMessage: (conversationId: string, msg: { role: Role; content: string }) => void
  refreshData: () => Promise<void>
}

const initialState: State = {
  projects: [
    { id: "unsorted", name: "Unsorted", icon: "folder", createdAt: Date.now() },
  ],
  conversations: [],
  currentConversationId: null,
  currentProjectId: typeof window !== "undefined" ? localStorage.getItem("currentProjectId") ?? null : null,
  loading: true,
  error: null,
}

const STORAGE_KEY = "prometheos_chat_state_v1"

type Action =
  | { type: "SET_LOADING"; loading: boolean }
  | { type: "SET_ERROR"; error: string | null }
  | { type: "LOAD"; payload: State }
  | { type: "SET_CURRENT"; id: string | null }
  | { type: "SET_CURRENT_PROJECT"; id: string | null }
  | { type: "UPSERT_PROJECT"; project: ChatProject }
  | { type: "RENAME_PROJECT"; id: string; name: string }
  | { type: "SET_PROJECT_ICON"; id: string; icon: string }
  | { type: "DELETE_PROJECT"; id: string }
  | { type: "UPSERT_CONV"; conv: ChatConversation }
  | { type: "RENAME_CONV"; id: string; title: string }
  | { type: "SET_CONV_ICON"; id: string; icon: string }
  | { type: "DELETE_CONV"; id: string }
  | { type: "MOVE_CONV"; id: string; projectId: string | null }
  | { type: "ADD_MESSAGE"; id: string; message: ChatMessage }

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case "SET_LOADING":
      return { ...state, loading: action.loading }
    case "SET_ERROR":
      return { ...state, error: action.error }
    case "LOAD":
      return action.payload
    case "SET_CURRENT":
      return { ...state, currentConversationId: action.id }
    case "SET_CURRENT_PROJECT":
      return { ...state, currentProjectId: action.id }
    case "UPSERT_PROJECT": {
      const exists = state.projects.some((p) => p.id === action.project.id)
      return {
        ...state,
        projects: exists
          ? state.projects.map((p) => (p.id === action.project.id ? action.project : p))
          : [...state.projects, action.project],
      }
    }
    case "RENAME_PROJECT": {
      return {
        ...state,
        projects: state.projects.map((p) => (p.id === action.id ? { ...p, name: action.name } : p)),
      }
    }
    case "SET_PROJECT_ICON": {
      return {
        ...state,
        projects: state.projects.map((p) => (p.id === action.id ? { ...p, icon: action.icon } : p)),
      }
    }
    case "DELETE_PROJECT": {
      return {
        ...state,
        projects: state.projects.filter((p) => p.id !== action.id),
        conversations: state.conversations.map((c) =>
          c.projectId === action.id ? { ...c, projectId: null } : c
        ),
        currentProjectId: state.currentProjectId === action.id ? null : state.currentProjectId,
      }
    }
    case "UPSERT_CONV": {
      const exists = state.conversations.some((c) => c.id === action.conv.id)
      return {
        ...state,
        conversations: exists
          ? state.conversations.map((c) => (c.id === action.conv.id ? action.conv : c))
          : [...state.conversations, action.conv],
      }
    }
    case "RENAME_CONV": {
      return {
        ...state,
        conversations: state.conversations.map((c) => (c.id === action.id ? { ...c, title: action.title } : c)),
      }
    }
    case "SET_CONV_ICON": {
      return {
        ...state,
        conversations: state.conversations.map((c) => (c.id === action.id ? { ...c, icon: action.icon } : c)),
      }
    }
    case "DELETE_CONV": {
      return {
        ...state,
        conversations: state.conversations.filter((c) => c.id !== action.id),
        currentConversationId: state.currentConversationId === action.id ? null : state.currentConversationId,
      }
    }
    case "MOVE_CONV": {
      return {
        ...state,
        conversations: state.conversations.map((c) =>
          c.id === action.id ? { ...c, projectId: action.projectId } : c
        ),
      }
    }
    case "ADD_MESSAGE": {
      return {
        ...state,
        conversations: state.conversations.map((c) =>
          c.id === action.id
            ? {
                ...c,
                messages: [...c.messages, action.message],
                updatedAt: Date.now(),
              }
            : c
        ),
      }
    }
    default:
      return state
  }
}

const ChatContext = createContext<ChatContextType | null>(null)

export function ChatProvider({ children }: { children: React.ReactNode }) {
  const [state, dispatch] = useReducer(reducer, initialState)

  // Load data from API on mount
  const refreshData = useCallback(async () => {
    dispatch({ type: "SET_LOADING", loading: true })
    dispatch({ type: "SET_ERROR", error: null })
    try {
      const projectsData = await getProjects()

      const projects: ChatProject[] = [
        { id: "unsorted", name: "Unsorted", icon: "folder", createdAt: Date.now() },
        ...projectsData.map((p) => ({
          id: p.id,
          name: p.name,
          icon: "folder",
          createdAt: new Date(p.created_at).getTime(),
        })),
      ]

      // Load conversations for each project
      const allConversations: ChatConversation[] = []
      for (const project of projectsData) {
        try {
          const convs = await getConversations(project.id)
          for (const conv of convs) {
            const messages = await getMessages(conv.id)
            allConversations.push({
              id: conv.id,
              title: conv.title,
              icon: "message-square",
              projectId: conv.project_id,
              messages: messages.map((m) => ({
                id: m.id,
                role: m.role as Role,
                content: m.content,
                createdAt: new Date(m.created_at).getTime(),
              })),
              createdAt: new Date(conv.created_at).getTime(),
              updatedAt: new Date(conv.updated_at).getTime(),
            })
          }
        } catch (e) {
          console.error(`Failed to load conversations for project ${project.id}:`, e)
        }
      }

      dispatch({
        type: "LOAD",
        payload: {
          ...initialState,
          projects,
          conversations: allConversations,
          loading: false,
        },
      })
    } catch (error) {
      console.error("Failed to load data:", error)
      dispatch({ type: "SET_ERROR", error: "Failed to load data" })
      dispatch({ type: "SET_LOADING", loading: false })
    }
  }, [])

  useEffect(() => {
    refreshData()
  }, [refreshData])

  // Persist to localStorage
  useEffect(() => {
    const dataToSave = {
      projects: state.projects,
      conversations: state.conversations,
      currentConversationId: state.currentConversationId,
    }
    localStorage.setItem(STORAGE_KEY, JSON.stringify(dataToSave))
  }, [state.projects, state.conversations, state.currentConversationId])

  const createProject = useCallback(async (opts?: { name?: string; icon?: string }) => {
    const projectName = opts?.name || `Project ${state.projects.length + 1}`
    const icon = opts?.icon || "folder"
    try {
      const backendProject = await createProjectApi(projectName)
      const newProject: ChatProject = {
        id: backendProject.id,
        name: backendProject.name,
        icon,
        createdAt: new Date(backendProject.created_at).getTime(),
      }
      dispatch({ type: "UPSERT_PROJECT", project: newProject })
    } catch (error) {
      console.error("Failed to create project:", error)
      const newProject: ChatProject = {
        id: `proj-${Date.now()}`,
        name: projectName,
        icon,
        createdAt: Date.now(),
      }
      dispatch({ type: "UPSERT_PROJECT", project: newProject })
    }
  }, [state.projects.length])

  const renameProject = useCallback((id: string, name: string) => {
    dispatch({ type: "RENAME_PROJECT", id, name })
  }, [])

  const setProjectIcon = useCallback((id: string, icon: string) => {
    dispatch({ type: "SET_PROJECT_ICON", id, icon })
  }, [])

  const deleteProject = useCallback((id: string) => {
    if (id === "unsorted") return
    dispatch({ type: "DELETE_PROJECT", id })
  }, [])

  const createConversation = useCallback(async (opts?: { title?: string; projectId?: string | null; icon?: string }) => {
    const title = opts?.title || `Chat ${state.conversations.length + 1}`
    const projectId = opts?.projectId ?? "unsorted"
    const icon = opts?.icon || "message-square"
    
    try {
      // Create conversation in backend
      const backendConv = await createConversationApi(projectId, title)
      
      const newConversation: ChatConversation = {
        id: backendConv.id,
        title: backendConv.title,
        icon,
        projectId: backendConv.project_id,
        messages: [],
        createdAt: new Date(backendConv.created_at).getTime(),
        updatedAt: new Date(backendConv.updated_at).getTime(),
      }
      dispatch({ type: "UPSERT_CONV", conv: newConversation })
      dispatch({ type: "SET_CURRENT", id: newConversation.id })
      return newConversation.id
    } catch (error) {
      console.error("Failed to create conversation:", error)
      // Fallback to local creation if backend fails
      const newConversation: ChatConversation = {
        id: `conv-${Date.now()}`,
        title,
        icon,
        projectId,
        messages: [],
        createdAt: Date.now(),
        updatedAt: Date.now(),
      }
      dispatch({ type: "UPSERT_CONV", conv: newConversation })
      dispatch({ type: "SET_CURRENT", id: newConversation.id })
      return newConversation.id
    }
  }, [state.conversations.length])

  const addMessage = useCallback((conversationId: string, msg: { role: Role; content: string }) => {
    const message: ChatMessage = {
      id: `msg-${Date.now()}`,
      role: msg.role,
      content: msg.content,
      createdAt: Date.now(),
    }
    dispatch({ type: "ADD_MESSAGE", id: conversationId, message })
  }, [])

  const renameConversation = useCallback((id: string, title: string) => {
    dispatch({ type: "RENAME_CONV", id, title })
  }, [])

  const setConversationIcon = useCallback((id: string, icon: string) => {
    dispatch({ type: "SET_CONV_ICON", id, icon })
  }, [])

  const deleteConversation = useCallback((id: string) => {
    dispatch({ type: "DELETE_CONV", id })
  }, [])

  const archiveConversation = useCallback((id: string) => {
    // For now, just delete - can be extended to have an archived state
    dispatch({ type: "DELETE_CONV", id })
  }, [])

  const duplicateConversation = useCallback((id: string) => {
    const conv = state.conversations.find(c => c.id === id)
    if (!conv) return
    const newConv: ChatConversation = {
      ...conv,
      id: `conv-${Date.now()}`,
      title: `${conv.title} (copy)`,
      messages: [],
      createdAt: Date.now(),
      updatedAt: Date.now(),
    }
    dispatch({ type: "UPSERT_CONV", conv: newConv })
  }, [state.conversations])

  const duplicateProject = useCallback((id: string) => {
    const project = state.projects.find(p => p.id === id)
    if (!project || id === "unsorted") return
    const newProject: ChatProject = {
      ...project,
      id: `proj-${Date.now()}`,
      name: `${project.name} (copy)`,
      createdAt: Date.now(),
    }
    dispatch({ type: "UPSERT_PROJECT", project: newProject })
  }, [state.projects])

  const archiveProject = useCallback((id: string) => {
    // For now, just delete - can be extended to have an archived state
    if (id === "unsorted") return
    dispatch({ type: "DELETE_PROJECT", id })
  }, [])

  const setCurrentConversation = useCallback((id: string | null) => {
    dispatch({ type: "SET_CURRENT", id })
  }, [])

  const setCurrentProject = useCallback((id: string | null) => {
    dispatch({ type: "SET_CURRENT_PROJECT", id })
    if (typeof window !== "undefined") {
      if (id) {
        localStorage.setItem("currentProjectId", id)
      } else {
        localStorage.removeItem("currentProjectId")
      }
    }
  }, [])

  const currentConversation = useMemo(
    () => state.conversations.find((c) => c.id === state.currentConversationId) || null,
    [state.conversations, state.currentConversationId]
  )

  return (
    <ChatContext.Provider
      value={{
        ...state,
        currentConversation,
        createProject,
        renameProject,
        setProjectIcon,
        deleteProject,
        archiveProject,
        duplicateProject,
        createConversation,
        renameConversation,
        setConversationIcon,
        deleteConversation,
        archiveConversation,
        duplicateConversation,
        setCurrentConversation,
        setCurrentProject,
        addMessage,
        refreshData,
      }}
    >
      {children}
    </ChatContext.Provider>
  )
}

export function useChat() {
  const context = useContext(ChatContext)
  if (!context) {
    throw new Error("useChat must be used within a ChatProvider")
  }
  return context
}
