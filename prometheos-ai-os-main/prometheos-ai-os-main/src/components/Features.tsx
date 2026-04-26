import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import techIcons from "@/assets/icons/tech_icons.png";

const Features = () => {
  const features = [
    {
      title: "Brain AI",
      tagline: "Post-transformer cognition",
      body: "A Rust-native, self-growing mind with multi-layer memory, symbolic planning and sub-150 ms latency.",
      icon: "🧠"
    },
    {
      title: "SOMA++",
      tagline: "Symbolic communication", 
      body: "A formal language that lets agents exchange Δ-phase packets for transparent reflection and control.",
      icon: "🔮"
    },
    {
      title: "Eidolon TTS",
      tagline: "Emotion-aware voice",
      body: "CNF-driven synthesis with mentor harmonics and real-time interrupt handling.",
      icon: "🎵"
    }
  ];

  return (
    <section id="features" className="py-20 bg-secondary/30">
      <div className="container mx-auto px-6">
        <div className="grid md:grid-cols-3 gap-8">
          {features.map((feature, index) => (
            <Card 
              key={index} 
              className="border-border/50 hover:border-primary/20 hover:bg-card-hover transition-all duration-300 group"
            >
              <CardHeader className="text-center">
                <div className="text-4xl mb-4 group-hover:scale-110 transition-transform duration-300">
                  {feature.icon}
                </div>
                <CardTitle className="font-headline text-2xl">{feature.title}</CardTitle>
                <CardDescription className="text-primary font-medium">
                  {feature.tagline}
                </CardDescription>
              </CardHeader>
              <CardContent>
                <p className="text-muted-foreground leading-relaxed">{feature.body}</p>
              </CardContent>
            </Card>
          ))}
        </div>
      </div>
    </section>
  );
};

export default Features;