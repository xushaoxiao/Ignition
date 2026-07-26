/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // Game skins are shipped as TS source in the workspace package; Next must transpile them.
  transpilePackages: ['@ignition/games'],
};

export default nextConfig;
