const Architecture = () => {
  const steps = [
    { title: "Input Layer", detail: "Voice, text, biometrics" },
    { title: "SOMBUCA Fusion", detail: "Multimodal emotion vector (ELV) generation" },
    { title: "Brain AI Core", detail: "Reasoning, memory, MuBrain planning" },
    { title: "Mentor Ensemble", detail: "Marcus, Seneca, Epictetus & custom LoRAs" },
    { title: "Prometheus Mode", detail: "Mixture-of-Experts meta-orchestrator" },
    { title: "Eidolon Output", detail: "Symbolic-to-Sonic compiler → audio stream" }
  ];

  return (
    <section id="architecture" className="py-20">
      <div className="container mx-auto px-6">
        <div className="text-center mb-16">
          <h2 className="font-headline text-4xl lg:text-5xl font-bold mb-4">
            System Architecture
          </h2>
          <p className="text-xl text-muted-foreground max-w-2xl mx-auto">
            From input to insight in six intelligent layers
          </p>
        </div>

        <div className="max-w-4xl mx-auto">
          <div className="relative">
            {/* Timeline line */}
            <div className="absolute left-8 top-8 bottom-8 w-0.5 bg-border"></div>
            
            {steps.map((step, index) => (
              <div key={index} className="relative flex items-start mb-12 last:mb-0">
                {/* Timeline dot */}
                <div className="relative z-10 w-16 h-16 bg-primary rounded-full flex items-center justify-center timeline-dot flex-shrink-0">
                  <span className="text-primary-foreground font-bold text-lg">
                    {index + 1}
                  </span>
                </div>
                
                {/* Content */}
                <div className="ml-8 flex-1">
                  <div className="bg-card border border-border/50 rounded-lg p-6 hover:border-primary/20 transition-colors">
                    <h3 className="font-headline text-xl font-semibold mb-2">
                      {step.title}
                    </h3>
                    <p className="text-muted-foreground">
                      {step.detail}
                    </p>
                  </div>
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
};

export default Architecture;