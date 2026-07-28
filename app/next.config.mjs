/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // The wallet adapters reach for Node built-ins that do not exist in a browser.
  // Telling webpack they resolve to nothing is the documented fix and is cheaper
  // than shipping polyfills for code paths the browser build never takes.
  webpack: (config) => {
    config.resolve.fallback = { ...config.resolve.fallback, fs: false, path: false, os: false };
    return config;
  },
};

export default nextConfig;
