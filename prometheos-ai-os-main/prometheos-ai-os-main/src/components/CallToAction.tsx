import { Button } from "@/components/ui/button";

const CallToAction = () => {
  return (
    <section className="py-20 bg-primary text-primary-foreground">
      <div className="container mx-auto px-6 text-center">
        <h2 className="font-headline text-4xl lg:text-5xl font-bold mb-6">
          Build with PrometheOS today
        </h2>
        <p className="text-xl mb-8 max-w-2xl mx-auto opacity-90">
          Ship voice-first, emotionally intelligent applications without touching LLM plumbing.
        </p>
        <Button size="lg" variant="secondary" asChild className="px-8">
          <a href="/signup">Get API Key</a>
        </Button>
      </div>
    </section>
  );
};

export default CallToAction;