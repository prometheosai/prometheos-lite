const Logo = ({ className = "", size = "h-8 w-8" }: { className?: string; size?: string }) => {
  return (
    <div className={`${size} flex items-center justify-center ${className}`}>
      <img 
        src="/lovable-uploads/98f60d14-008f-429b-b2dd-7873075a25a0.png" 
        alt="PrometheOS Logo" 
        className="w-full h-full object-contain"
      />
    </div>
  );
};

export default Logo;