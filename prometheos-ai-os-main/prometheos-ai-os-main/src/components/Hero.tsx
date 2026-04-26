import { useEffect, useRef, useState } from "react";
import { useToast } from "@/hooks/use-toast";
import { supabase } from "@/integrations/supabase/client";
import ReactMarkdown from "react-markdown";
import { useAuth } from "@/hooks/useAuth";
import { Plus, Wrench, Mic, AudioLines, Send, Image as ImageIcon, FileText, Link2 } from "lucide-react";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogTrigger } from "@/components/ui/dialog";
import { useChat } from "@/store/chat";
const Hero = () => {
  const [message, setMessage] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [selectedMode, setSelectedMode] = useState<string | null>(null);
  const { toast } = useToast();
  const [isRecording, setIsRecording] = useState(false);
  const [uploadOpen, setUploadOpen] = useState(false);
  const [toolsOpen, setToolsOpen] = useState(false);
  const { currentConversation, addMessage, ensureActiveConversation } = useChat();
  const bottomRef = useRef<HTMLDivElement>(null);
  const { user } = useAuth();
  const meta = (user?.user_metadata as any) || {};
  const firstName = meta.first_name as string | undefined;
  const lastName = meta.last_name as string | undefined;
  const displayName = firstName && lastName ? `${firstName} ${lastName}` : firstName || (user?.email?.split("@")[0]);

  const prompts = [
    `Hey${displayName ? `, ${displayName}` : ""}. Ready to dive in?`,
    `What can I help you build today${displayName ? `, ${displayName}` : ""}?`,
    "Got a question or idea? Let's explore it together.",
    "Stuck on something? I can help break it down."
  ];
  const [starter, setStarter] = useState("");

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [currentConversation?.messages]);

  useEffect(() => {
    setStarter(prompts[Math.floor(Math.random() * prompts.length)]);
  }, [user?.id]);

  const handleSubmit = async (mode?: string) => {
    if (!message.trim()) return;
    const text = message.trim();

    // capture history BEFORE adding the new user message
    const history = (currentConversation?.messages || []).map(m => ({ role: m.role, content: m.content }));

    const convId = ensureActiveConversation({ title: text.slice(0, 40) });

    // Optimistic user message append
    addMessage(convId, { role: 'user', content: text });

    setIsLoading(true);
    try {
      const { data, error } = await supabase.functions.invoke('chat', {
        body: {
          message: text,
          mode: mode || selectedMode || 'general',
          history
        },
      });

      if (error) {
        throw new Error(error.message || 'Failed to get response');
      }

      const aiText = (data as any).response as string;
      addMessage(convId, { role: 'assistant', content: aiText });
      setMessage("");
      setSelectedMode(null);
    } catch (error) {
      console.error('Error:', error);
      toast({
        title: "Error",
        description: "Failed to get response from PrometheOS",
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  };

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  };

  const handleModeClick = (mode: string) => {
    if (message.trim()) {
      handleSubmit(mode);
    } else {
      setSelectedMode(mode);
      toast({
        title: `${mode.charAt(0).toUpperCase() + mode.slice(1)} mode selected`,
        description: "Type your message and I'll respond in this mode",
      });
    }
  };

  return (
    <div className="min-h-screen flex flex-col items-center justify-center p-6 bg-background">
      <div className="w-full max-w-2xl mx-auto">
        
        <div className="flex justify-center mb-16">
          <p className="text-2xl md:text-3xl font-medium text-center text-foreground/90">{starter || `Hey${displayName ? `, ${displayName}` : ""}. Ready to dive in?`}</p>
        </div>

        {/* Messages */}
        <div className="mb-8 space-y-4 max-h-[50vh] overflow-y-auto pr-1">
          {(currentConversation?.messages || []).map((m, idx) => (
            <article key={(m as any).id || idx} className={`p-4 rounded-2xl border ${m.role === 'user' ? 'bg-background' : 'bg-muted'}`}>
              <div className="text-foreground prose prose-sm max-w-none">
                <ReactMarkdown>{m.content}</ReactMarkdown>
              </div>
            </article>
          ))}
          <div ref={bottomRef} />
        </div>

        {/* Main Chat Input */}
        <div className="relative">
          <div className="flex flex-col gap-3 p-4 bg-background border rounded-[10px] shadow-sm focus-within:border-primary transition-colors">
            {/* Text Area */}
            <div>
              <textarea
                value={message}
                onChange={(e) => {
                  const el = e.currentTarget;
                  el.style.height = 'auto';
                  const max = 192; // 12rem max height
                  const newHeight = Math.min(el.scrollHeight, max);
                  el.style.height = newHeight + 'px';
                  el.style.overflowY = el.scrollHeight > max ? 'auto' : 'hidden';
                  setMessage(e.target.value);
                }}
                onKeyDown={handleKeyPress}
                placeholder="Ask anything"
                rows={1}
                className="w-full resize-none bg-transparent text-foreground placeholder:text-muted-foreground focus:outline-none leading-relaxed max-h-48 overflow-y-auto"
                disabled={isLoading}
              />
            </div>

            {/* Actions Row */}
            <div className="flex items-end justify-between">
              <div className="flex items-center gap-2">
                {/* Add/Plus Button */}
                <Dialog open={uploadOpen} onOpenChange={setUploadOpen}>
                  <DialogTrigger asChild>
                    <button onClick={() => setUploadOpen(true)} className="p-2 hover:bg-muted rounded-lg transition-colors hover-scale active:scale-95 focus-visible:ring-2 focus-visible:ring-primary/30" aria-label="Attach files">
                      <Plus className={`h-5 w-5 text-muted-foreground transition-transform duration-200 ${uploadOpen ? 'rotate-45 scale-110' : ''}`} />
                    </button>
                  </DialogTrigger>
                  <DialogContent className="sm:max-w-md">
                    <DialogHeader>
                      <DialogTitle>Attach to message</DialogTitle>
                      <DialogDescription>Upload files or add links for context.</DialogDescription>
                    </DialogHeader>
                    <div className="grid grid-cols-3 gap-3">
                      <button className="flex flex-col items-center gap-2 p-3 rounded-[10px] border hover:bg-muted transition-colors" aria-label="Upload image">
                        <ImageIcon className="h-5 w-5" />
                        <span className="text-xs">Image</span>
                      </button>
                      <button className="flex flex-col items-center gap-2 p-3 rounded-[10px] border hover:bg-muted transition-colors" aria-label="Upload document">
                        <FileText className="h-5 w-5" />
                        <span className="text-xs">Document</span>
                      </button>
                      <button className="flex flex-col items-center gap-2 p-3 rounded-[10px] border hover:bg-muted transition-colors" aria-label="Add link">
                        <Link2 className="h-5 w-5" />
                        <span className="text-xs">Link</span>
                      </button>
                    </div>
                  </DialogContent>
                </Dialog>
                {/* Tools Button */}
                <Dialog open={toolsOpen} onOpenChange={setToolsOpen}>
                  <DialogTrigger asChild>
                    <button 
                      onClick={() => setToolsOpen(true)}
                      disabled={isLoading}
                      className={`flex items-center gap-2 px-3 py-2 rounded-lg text-sm transition-colors hover-scale active:scale-95 focus-visible:ring-2 focus-visible:ring-primary/30 ${
                        toolsOpen 
                          ? 'bg-primary text-primary-foreground' 
                          : 'hover:bg-muted text-muted-foreground'
                      }`}
                    >
                      <Wrench className={`h-4 w-4 transition-transform duration-200 ${toolsOpen ? 'rotate-12 scale-110' : ''}`} />
                      <span>Tools</span>
                    </button>
                  </DialogTrigger>
                  <DialogContent className="sm:max-w-md">
                    <DialogHeader>
                      <DialogTitle>Tools</DialogTitle>
                      <DialogDescription>Choose utilities to enhance your prompt.</DialogDescription>
                    </DialogHeader>
                    <div className="grid grid-cols-2 gap-3">
                      <button className="p-3 rounded-[10px] border hover:bg-muted transition-colors text-sm">Summarize</button>
                      <button className="p-3 rounded-[10px] border hover:bg-muted transition-colors text-sm">Translate</button>
                      <button className="p-3 rounded-[10px] border hover:bg-muted transition-colors text-sm">Code</button>
                      <button className="p-3 rounded-[10px] border hover:bg-muted transition-colors text-sm">Brainstorm</button>
                    </div>
                  </DialogContent>
                </Dialog>
              </div>

              <div className="flex items-center gap-2">
                {/* Microphone */}
                <button 
                  type="button"
                  onClick={() => {
                    const next = !isRecording;
                    setIsRecording(next);
                    toast({ title: next ? "Recording…" : "Recording stopped" });
                  }}
                  disabled={isLoading}
                  aria-pressed={isRecording}
                  title={isRecording ? "Stop recording" : "Start recording"}
                  className={`p-2 rounded-lg transition-colors hover-scale active:scale-95 focus-visible:ring-2 focus-visible:ring-primary/30 ${isRecording ? 'bg-foreground text-background' : 'hover:bg-muted'}`}
                >
                  <Mic className="h-5 w-5" />
                </button>
                {/* Live AI Call */}
                <button 
                  disabled={isLoading}
                  title="Start live AI call"
                  className="group p-2 hover:bg-muted rounded-lg transition-colors hover-scale active:scale-95 focus-visible:ring-2 focus-visible:ring-primary/30"
                >
                  <AudioLines className="h-5 w-5 text-muted-foreground group-hover:animate-pulse" />
                </button>
                {/* Send */}
                {message.trim() && (
                  <button
                    onClick={() => handleSubmit()}
                    disabled={isLoading}
                    className="p-2 bg-primary hover:bg-primary/90 text-primary-foreground rounded-lg transition-colors hover-scale active:scale-95 focus-visible:ring-2 focus-visible:ring-primary/30"
                  >
                    <Send className="h-4 w-4" />
                  </button>
                )}
              </div>
            </div>
          </div>
        </div>

        {/* Mode Selection Indicator */}
        {selectedMode && selectedMode !== 'tools' && (
          <div className="mt-4 text-center">
            <span className="inline-flex items-center px-3 py-1 rounded-full text-sm bg-primary/10 text-primary">
              {selectedMode.charAt(0).toUpperCase() + selectedMode.slice(1)} mode active
            </span>
          </div>
        )}

        {/* Loading State */}
        {isLoading && (
          <div className="mt-6 text-center">
            <div className="inline-flex items-center px-4 py-2 text-sm text-muted-foreground">
              <div className="w-4 h-4 border-2 border-primary border-t-transparent rounded-full animate-spin mr-2"></div>
              PrometheOS is thinking...
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

export default Hero;