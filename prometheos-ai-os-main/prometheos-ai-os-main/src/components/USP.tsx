const USP = () => {
  const features = [
    "⚡ Ultra-low latency (<150 ms round-trip)",
    "🧠 Self-growing symbolic memory",
    "💬 Interruptible real-time dialogue",
    "🎭 Emotionally recursive mentor blending",
    "🔒 Containerised per-user privacy with global knowledge distillation",
    "🛠 Modular micro-service architecture (Rust + WASM)",
    "🕊 Open, extensible SOMA++ protocol"
  ];

  return (
    <section className="py-20 bg-accent-subtle/50">
      <div className="container mx-auto px-6">
        <div className="text-center mb-12">
          <h2 className="font-headline text-4xl lg:text-5xl font-bold mb-4">
            Why PrometheOS?
          </h2>
          <p className="text-xl text-muted-foreground">
            Built for the future of human-AI collaboration
          </p>
        </div>

        <div className="max-w-4xl mx-auto">
          <div className="grid md:grid-cols-2 gap-6">
            {features.map((feature, index) => (
              <div 
                key={index}
                className="flex items-start gap-4 p-6 bg-card border border-border/50 rounded-lg hover:border-primary/20 transition-colors group"
              >
                <span className="text-2xl group-hover:scale-110 transition-transform duration-300">
                  {feature.charAt(0)}
                </span>
                <span className="text-foreground font-medium flex-1">
                  {feature.slice(2)}
                </span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
};

export default USP;