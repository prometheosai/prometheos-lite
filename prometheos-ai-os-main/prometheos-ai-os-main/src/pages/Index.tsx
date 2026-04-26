
import { SidebarProvider } from "@/components/ui/sidebar";
import Hero from "@/components/Hero";
import ChatSidebar from "@/components/chat/ChatSidebar";
import Logo from "@/components/Logo";
import { ChatProvider } from "@/store/chat";
import { useAuth } from "@/hooks/useAuth";



const AuthenticatedHeader = () => {
  
  
  return (
    <header className="relative z-50 h-12 flex items-center px-2 bg-background">
      <div className="flex items-center gap-2">
        <Logo size="h-7 w-7" />
        <h1 className="sr-only">PrometheOS Chat – AI Assistant</h1>
      </div>
    </header>
  );
};

const UnauthenticatedHeader = () => {
  return (
    <header className="relative z-50 h-12 flex items-center justify-between px-2 bg-background">
      <div className="flex items-center gap-2">
        <Logo size="h-7 w-7" />
        <h1 className="sr-only">PrometheOS Chat – AI Assistant</h1>
      </div>
      <a href="/auth" className="text-sm text-muted-foreground hover:text-foreground px-3 py-1">
        Sign in
      </a>
    </header>
  );
};

const Index = () => {
  const { user, loading } = useAuth();

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <div className="w-4 h-4 border-2 border-primary border-t-transparent rounded-full animate-spin"></div>
      </div>
    );
  }

  return (
    <ChatProvider>
      <SidebarProvider>
        {user ? <AuthenticatedHeader /> : <UnauthenticatedHeader />}
        <div className="flex w-full min-h-[calc(100vh-3rem)]">
          <ChatSidebar />
          <main className="flex-1">
            <Hero />
          </main>
        </div>
      </SidebarProvider>
    </ChatProvider>
  );
};

export default Index;
