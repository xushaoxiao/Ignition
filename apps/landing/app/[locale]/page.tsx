import { notFound } from "next/navigation";
import { Cta } from "@/components/sections/cta";
import { Faq } from "@/components/sections/faq";
import { Features } from "@/components/sections/features";
import { Hero } from "@/components/sections/hero";
import { HowItWorks } from "@/components/sections/how-it-works";
import { Positioning } from "@/components/sections/positioning";
import { Pricing } from "@/components/sections/pricing";
import { Trust } from "@/components/sections/trust";
import { SiteFooter } from "@/components/site-footer";
import { SiteHeader } from "@/components/site-header";
import { isLocale } from "@/i18n/config";
import { getDictionary } from "@/i18n/dictionaries";

export default async function Page({ params }: { params: Promise<{ locale: string }> }) {
  const { locale } = await params;
  if (!isLocale(locale)) notFound();
  const dict = getDictionary(locale);

  return (
    <>
      <SiteHeader dict={dict} locale={locale} />
      <main>
        <Hero dict={dict.hero} />
        <Positioning dict={dict.positioning} />
        <Features dict={dict.features} />
        <HowItWorks dict={dict.how} />
        <Trust dict={dict.trust} />
        <Pricing dict={dict.pricing} />
        <Faq dict={dict.faq} />
        <Cta dict={dict.cta} />
      </main>
      <SiteFooter dict={dict.footer} />
    </>
  );
}
