import { Button } from "@/components/ui/button";
import Logo from "@/components/Logo";

const Navigation = () => {
  return (
    <nav className="sticky top-0 z-50 bg-background/80 backdrop-blur-md border-b">
      <div className="container mx-auto px-6 py-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center space-x-2">
            <Logo size="h-8 w-8" className="text-foreground" />
            <span className="font-headline text-xl font-semibold">PrometheOS™</span>
          </div>
          
          <div className="hidden md:flex items-center space-x-6">
            <a href="/docs" className="text-foreground hover:text-primary transition-colors">
              Docs
            </a>
            <a href="/roadmap" className="text-foreground hover:text-primary transition-colors">
              Roadmap
            </a>
            <a href="/blog" className="text-foreground hover:text-primary transition-colors">
              Blog
            </a>
            <a href="https://github.com/mementomori-labs/prometheos" className="text-foreground hover:text-primary transition-colors">
              GitHub
            </a>
          </div>

          <div className="flex items-center space-x-3">
            <Button variant="ghost" asChild>
              <a href="/login">Sign in</a>
            </Button>
            <Button asChild>
              <a href="/signup">Early Access</a>
            </Button>
          </div>
        </div>
      </div>
    </nav>
  );
};

export default Navigation;