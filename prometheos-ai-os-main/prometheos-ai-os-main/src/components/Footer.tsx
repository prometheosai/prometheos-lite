const Footer = () => {
  const footerColumns = [
    {
      heading: "Product",
      links: [
        { label: "Pricing", href: "/pricing" },
        { label: "Changelog", href: "/changelog" },
        { label: "Status", href: "/status" }
      ]
    },
    {
      heading: "Developers",
      links: [
        { label: "API Reference", href: "/docs/api" },
        { label: "SDKs", href: "/docs/sdks" },
        { label: "Community", href: "https://discord.gg/prometheos" }
      ]
    },
    {
      heading: "Company",
      links: [
        { label: "About Memento Mori Labs", href: "/about" },
        { label: "Careers", href: "/careers" },
        { label: "Press", href: "/press" }
      ]
    }
  ];

  return (
    <footer className="bg-muted border-t">
      <div className="container mx-auto px-6 py-12">
        <div className="grid md:grid-cols-4 gap-8">
          <div className="md:col-span-1">
            <h3 className="font-headline text-xl font-semibold mb-4">PrometheOS™</h3>
            <p className="text-muted-foreground text-sm">
              Symbolic AI Operating System for the future of human-AI collaboration.
            </p>
          </div>
          
          {footerColumns.map((column, index) => (
            <div key={index}>
              <h4 className="font-semibold mb-4">{column.heading}</h4>
              <ul className="space-y-2">
                {column.links.map((link, linkIndex) => (
                  <li key={linkIndex}>
                    <a 
                      href={link.href}
                      className="text-muted-foreground hover:text-foreground transition-colors text-sm"
                    >
                      {link.label}
                    </a>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
        
        <div className="border-t mt-12 pt-8">
          <p className="text-center text-muted-foreground text-sm">
            © 2025 PrometheOS. All rights reserved.
          </p>
        </div>
      </div>
    </footer>
  );
};

export default Footer;