import React, { createContext, useCallback, useContext, useEffect, useMemo, useReducer } from "react";

export type Role = "user" | "assistant" | "system";

export type Message = {
  id: string;
  role: Role;
  content: string;
  createdAt: number;
};

export type Conversation = {
  id: string;
  title: string;
  icon: string; // lucide icon key
  projectId: string | null;
  messages: Message[];
  createdAt: number;
  updatedAt: number;
};

export type Project = {
  id: string;
  name: string;
  icon: string; // lucide icon key
  createdAt: number;
};

type State = {
  projects: Project[];
  conversations: Conversation[];
  currentConversationId: string | null;
};

type ChatContextType = State & {
  currentConversation: Conversation | null;
  createProject: (name?: string) => string;
  renameProject: (id: string, name: string) => void;
  setProjectIcon: (id: string, icon: string) => void;
  deleteProject: (id: string) => void;
  createConversation: (opts?: { title?: string; projectId?: string | null }) => string;
  renameConversation: (id: string, title: string) => void;
  setConversationIcon: (id: string, icon: string) => void;
  deleteConversation: (id: string) => void;
  moveConversationToProject: (id: string, projectId: string | null) => void;
  setCurrentConversation: (id: string | null) => void;
  addMessage: (conversationId: string, msg: { role: Role; content: string }) => void;
  ensureActiveConversation: (opts?: { title?: string; projectId?: string | null }) => string;
};

const initialState: State = {
  projects: [
    { id: "unsorted", name: "Unsorted", icon: "folder", createdAt: Date.now() },
  ],
  conversations: [],
  currentConversationId: null,
};

const STORAGE_KEY = "prometheos_chat_state_v1";

/* Reducer */

type Action =
  | { type: "LOAD"; payload: State }
  | { type: "SET_CURRENT"; id: string | null }
  | { type: "UPSERT_PROJECT"; project: Project }
  | { type: "RENAME_PROJECT"; id: string; name: string }
  | { type: "SET_PROJECT_ICON"; id: string; icon: string }
  | { type: "DELETE_PROJECT"; id: string }
  | { type: "UPSERT_CONV"; conv: Conversation }
  | { type: "RENAME_CONV"; id: string; title: string }
  | { type: "SET_CONV_ICON"; id: string; icon: string }
  | { type: "DELETE_CONV"; id: string }
  | { type: "MOVE_CONV"; id: string; projectId: string | null }
  | { type: "ADD_MESSAGE"; id: string; message: Message };

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case "LOAD":
      return action.payload;
    case "SET_CURRENT":
      return { ...state, currentConversationId: action.id };
    case "UPSERT_PROJECT": {
      const exists = state.projects.some((p) => p.id === action.project.id);
      return {
        ...state,
        projects: exists
          ? state.projects.map((p) => (p.id === action.project.id ? action.project : p))
          : [...state.projects, action.project],
      };
    }
    case "RENAME_PROJECT": {
      return {
        ...state,
        projects: state.projects.map((p) => (p.id === action.id ? { ...p, name: action.name } : p)),
      };
    }
    case "SET_PROJECT_ICON": {
      return {
        ...state,
        projects: state.projects.map((p) => (p.id === action.id ? { ...p, icon: action.icon } : p)),
      };
    }
    case "DELETE_PROJECT": {
      const projects = state.projects.filter((p) => p.id !== action.id);
      const conversations = state.conversations.map((c) =>
        c.projectId === action.id ? { ...c, projectId: "unsorted" } : c
      );
      return { ...state, projects, conversations };
    }
    case "UPSERT_CONV": {
      const exists = state.conversations.some((c) => c.id === action.conv.id);
      const conversations = exists
        ? state.conversations.map((c) => (c.id === action.conv.id ? action.conv : c))
        : [...state.conversations, action.conv];
      return { ...state, conversations };
    }
    case "RENAME_CONV": {
      const conversations = state.conversations.map((c) =>
        c.id === action.id ? { ...c, title: action.title, updatedAt: Date.now() } : c
      );
      return { ...state, conversations };
    }
    case "SET_CONV_ICON": {
      const conversations = state.conversations.map((c) =>
        c.id === action.id ? { ...c, icon: action.icon, updatedAt: Date.now() } : c
      );
      return { ...state, conversations };
    }
    case "DELETE_CONV": {
      const conversations = state.conversations.filter((c) => c.id !== action.id);
      const currentConversationId =
        state.currentConversationId === action.id ? null : state.currentConversationId;
      return { ...state, conversations, currentConversationId };
    }
    case "MOVE_CONV": {
      const conversations = state.conversations.map((c) =>
        c.id === action.id ? { ...c, projectId: action.projectId, updatedAt: Date.now() } : c
      );
      return { ...state, conversations };
    }
    case "ADD_MESSAGE": {
      const conversations = state.conversations.map((c) =>
        c.id === action.id
          ? {
              ...c,
              messages: [...c.messages, action.message],
              updatedAt: Date.now(),
            }
          : c
      );
      return { ...state, conversations };
    }
    default:
      return state;
  }
}

const ChatContext = createContext<ChatContextType | null>(null);

function createId() {
  if (typeof crypto !== "undefined" && crypto.randomUUID) return crypto.randomUUID();
  return Math.random().toString(36).slice(2);
}

export const ChatProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [state, dispatch] = useReducer(reducer, initialState);

  const currentConversation = useMemo(() => {
    return state.conversations.find((c) => c.id === state.currentConversationId) || null;
  }, [state.conversations, state.currentConversationId]);
  // Load from localStorage once
  useEffect(() => {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (raw) {
        const parsed = JSON.parse(raw) as State;
        // Ensure Unsorted exists
        const hasUnsorted = parsed.projects.some((p) => p.id === "unsorted");
        if (!hasUnsorted) parsed.projects.unshift({ id: "unsorted", name: "Unsorted", icon: "folder", createdAt: Date.now() });
        // Backfill missing fields
        parsed.projects = (parsed.projects || []).map((p: any) => ({
          ...p,
          icon: p.icon || "folder",
        }));
        parsed.conversations = (parsed.conversations || []).map((c: any) => ({
          ...c,
          icon: c.icon || "message-square",
        }));
        dispatch({ type: "LOAD", payload: parsed });
      }
    } catch (e) {
      console.warn("Failed to load chat state:", e);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Persist
  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    } catch (e) {
      console.warn("Failed to persist chat state:", e);
    }
  }, [state]);

  const createProject = useCallback((name = "New Project") => {
    const id = createId();
    dispatch({
      type: "UPSERT_PROJECT",
      project: { id, name, icon: "folder", createdAt: Date.now() },
    });
    return id;
  }, []);

  const renameProject = useCallback((id: string, name: string) => {
    dispatch({ type: "RENAME_PROJECT", id, name });
  }, []);

  const setProjectIcon = useCallback((id: string, icon: string) => {
    dispatch({ type: "SET_PROJECT_ICON", id, icon });
  }, []);

  const deleteProject = useCallback((id: string) => {
    if (id === "unsorted") return; // keep default
    dispatch({ type: "DELETE_PROJECT", id });
  }, []);

  const createConversation = useCallback((opts?: { title?: string; projectId?: string | null }) => {
    const id = createId();
    const conv: Conversation = {
      id,
      title: opts?.title || "New chat",
      icon: "message-square",
      projectId: opts?.projectId ?? "unsorted",
      messages: [],
      createdAt: Date.now(),
      updatedAt: Date.now(),
    };
    dispatch({ type: "UPSERT_CONV", conv });
    dispatch({ type: "SET_CURRENT", id });
    return id;
  }, []);

  const renameConversation = useCallback((id: string, title: string) => {
    dispatch({ type: "RENAME_CONV", id, title });
  }, []);

  const deleteConversation = useCallback((id: string) => {
    dispatch({ type: "DELETE_CONV", id });
  }, []);

  const setConversationIcon = useCallback((id: string, icon: string) => {
    dispatch({ type: "SET_CONV_ICON", id, icon });
  }, []);

  const moveConversationToProject = useCallback((id: string, projectId: string | null) => {
    dispatch({ type: "MOVE_CONV", id, projectId });
  }, []);

  const setCurrentConversation = useCallback((id: string | null) => {
    dispatch({ type: "SET_CURRENT", id });
  }, []);

  const addMessage = useCallback((conversationId: string, msg: { role: Role; content: string }) => {
    const message: Message = { id: createId(), createdAt: Date.now(), ...msg };
    dispatch({ type: "ADD_MESSAGE", id: conversationId, message });
  }, []);

  const ensureActiveConversation = useCallback((opts?: { title?: string; projectId?: string | null }) => {
    if (state.currentConversationId) return state.currentConversationId;
    return createConversation(opts);
  }, [state.currentConversationId, createConversation]);

  const value: ChatContextType = useMemo(
    () => ({
      ...state,
      currentConversation,
      createProject,
      renameProject,
      setProjectIcon,
      deleteProject,
      createConversation,
      renameConversation,
      setConversationIcon,
      deleteConversation,
      moveConversationToProject,
      setCurrentConversation,
      addMessage,
      ensureActiveConversation,
    }),
    [state, currentConversation, createProject, renameProject, setProjectIcon, deleteProject, createConversation, renameConversation, setConversationIcon, deleteConversation, moveConversationToProject, setCurrentConversation, addMessage, ensureActiveConversation]
  );

  return <ChatContext.Provider value={value}>{children}</ChatContext.Provider>;
};

export const useChat = () => {
  const ctx = useContext(ChatContext);
  if (!ctx) throw new Error("useChat must be used within ChatProvider");
  return ctx;
};
