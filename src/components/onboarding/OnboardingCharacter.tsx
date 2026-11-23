import { useId } from "react";
import { CLOVY_BODY_PATH, CLOVY_EYES_PATH } from "../brand/ClovyLogo";
import type { OnboardingMood } from "../../lib/onboarding";
import { type MessageKey, t, useT } from "../../i18n";

const CHARACTER_VIEWBOX = "-18 -18 293 300";
const CALM_EYES_PATH =
  "M15.6397 0C19.1397 0 30.1397 1.50002 31.1397 9.5C32.1397 17.5 27.6397 15 24.1397 13.5C21.3403 12.3003 17.3079 12.0001 15.6407 12C13.9733 12.0001 9.94099 12.3003 7.14168 13.5C3.64171 15 -0.858285 17.4998 0.14168 9.5C1.14147 1.50168 12.1373 0.000628685 15.6397 0ZM55.6397 0C59.1397 0 70.1397 1.50002 71.1397 9.5C72.1397 17.5 67.6397 15 64.1397 13.5C61.3403 12.3003 57.3079 12.0001 55.6407 12C53.9733 12.0001 49.941 12.3003 47.1417 13.5C43.6417 15 39.1417 17.4998 40.1417 9.5C41.1415 1.50168 52.1373 0.000628685 55.6397 0Z";
const QUICK_WITTED_EYES_PATHS = [
  "M54.6397 13C58.1397 13 69.1397 14.5 70.1397 22.5C71.1397 30.5 66.6397 28 63.1397 26.5C60.3403 25.3002 56.308 25.0001 54.6407 25C52.9734 25.0001 48.9411 25.3003 46.1417 26.5C42.6417 28 38.1417 30.4999 39.1417 22.5C40.1415 14.5016 51.1375 13.0006 54.6397 13Z",
  "M0 22.5C0 10.0737 6.10097 0.000149773 13.627 0C21.1529 0.00024739 27.2539 10.0738 27.2539 22.5C27.2539 28.1141 26.008 33.2472 23.9482 37.1885C22.9459 39.1063 20.3336 38.747 18.7627 37.2588C17.2563 35.8316 15.4464 35 13.5 35C11.6209 35 9.86967 35.7754 8.39551 37.1133C6.79213 38.5685 4.1706 38.8934 3.18848 36.9639C1.19867 33.053 0 28.0074 0 22.5Z",
] as const;
const STRATEGIC_EYES_PATHS = [
  "M76.4366 42.1564C77.2296 29.3676 85.5245 19.4745 94.9639 20.0597C104.403 20.645 111.413 31.4876 110.62 44.2765C109.827 57.0652 101.532 66.9583 92.0928 66.3732C82.6533 65.788 75.6437 54.9454 76.4366 42.1564Z",
  "M5.87601 39.5314C6.67893 26.5806 15.0798 16.5618 24.6387 17.1545C34.1974 17.7474 41.2951 28.7263 40.4922 41.6769C39.6893 54.6278 31.2895 64.6465 21.7305 64.0539C12.1717 63.4612 5.0733 52.482 5.87601 39.5314Z",
] as const;
const STRATEGIC_EYEBROW_PATHS = [
  "M76.6221 7.70328C81.2794 5.99253 88.4256 4.82465 96.2891 5.02164C104.189 5.21961 113.135 6.80261 121.29 10.88C123.266 11.868 124.067 14.2705 123.079 16.2463C122.091 18.2222 119.688 19.0232 117.712 18.0353C110.868 14.6136 103.146 13.1965 96.0879 13.0197C88.9935 12.842 82.8892 13.9239 79.3799 15.213C77.3064 15.9745 75.0078 14.9106 74.2461 12.8371C73.4848 10.7636 74.5487 8.46497 76.6221 7.70328Z",
  "M2.6221 2.70328C7.2794 0.992527 14.4256 -0.175349 22.2891 0.0216435C30.1891 0.219611 39.1345 1.80261 47.2901 5.88004C49.2657 6.86802 50.0668 9.27048 49.0791 11.2463C48.0912 13.2222 45.6879 14.0232 43.7119 13.0353C36.8677 9.61356 29.1459 8.19652 22.0879 8.01969C14.9935 7.84201 8.88919 8.92393 5.37992 10.213C3.30637 10.9745 1.00783 9.91059 0.246128 7.83707C-0.515164 5.7636 0.548693 3.46497 2.6221 2.70328Z",
] as const;

type MoodPresentation = {
  label: string;
  description: string;
  greeting: string;
};

// Getters so each read follows the current interface language (Settings reads
// this map too). The English values are the canonical copy.
function moodPresentation(keys: Record<keyof MoodPresentation, MessageKey>): MoodPresentation {
  return {
    get label() {
      return t(keys.label);
    },
    get description() {
      return t(keys.description);
    },
    get greeting() {
      return t(keys.greeting);
    },
  };
}

export const ONBOARDING_MOOD_PRESENTATION: Record<OnboardingMood, MoodPresentation> = {
  calm: moodPresentation({
    label: "onboarding.mood.calm.label",
    description: "onboarding.mood.calm.description",
    greeting: "onboarding.mood.calm.greeting",
  }),
  clearheaded: moodPresentation({
    label: "onboarding.mood.clearheaded.label",
    description: "onboarding.mood.clearheaded.description",
    greeting: "onboarding.mood.clearheaded.greeting",
  }),
  "quick-witted": moodPresentation({
    label: "onboarding.mood.quickWitted.label",
    description: "onboarding.mood.quickWitted.description",
    greeting: "onboarding.mood.quickWitted.greeting",
  }),
  strategic: moodPresentation({
    label: "onboarding.mood.strategic.label",
    description: "onboarding.mood.strategic.description",
    greeting: "onboarding.mood.strategic.greeting",
  }),
};

export function OnboardingCharacter({
  mood,
  className,
}: {
  mood: OnboardingMood;
  className?: string;
}) {
  const translate = useT();
  const gradientId = `clovy-personality-${useId().replaceAll(":", "")}`;

  return (
    <span
      className={`onboarding-character${className ? ` ${className}` : ""}`}
      data-mood={mood}
      aria-hidden="true"
    >
      <svg
        className="onboarding-character-art"
        viewBox={CHARACTER_VIEWBOX}
        preserveAspectRatio="xMidYMid meet"
        focusable="false"
      >
        <title>
          {translate("onboarding.mood.characterTitle", {
            mood: ONBOARDING_MOOD_PRESENTATION[mood].label,
          })}
        </title>
        <defs>
          <linearGradient
            id={gradientId}
            x1="128.5"
            x2="128.5"
            y1="0"
            y2="264"
            gradientUnits="userSpaceOnUse"
          >
            <stop stopColor="var(--clovy-glow-top)" />
            <stop offset="1" stopColor="var(--clovy-glow)" />
          </linearGradient>
        </defs>
        {/* Keyed by mood so a selection swaps the expression in one commit and
         * replays the greeting gesture; rapid changes remount cleanly. */}
        <g key={mood} className="onboarding-character-body">
          <path
            className="onboarding-character-depth"
            d={CLOVY_BODY_PATH}
            transform="translate(0 5)"
          />
          <path
            className="onboarding-character-fill"
            d={CLOVY_BODY_PATH}
            fill={`url(#${gradientId})`}
          />
          <path className="onboarding-character-rim" d={CLOVY_BODY_PATH} />
          <CharacterEyes mood={mood} />
        </g>
      </svg>
    </span>
  );
}

function CharacterEyes({ mood }: { mood: OnboardingMood }) {
  if (mood === "calm") {
    return (
      <g className="onboarding-character-eyes onboarding-character-eyes-calm">
        <path d={CALM_EYES_PATH} transform="translate(91.9873 126)" />
      </g>
    );
  }

  if (mood === "quick-witted") {
    return (
      <g className="onboarding-character-eyes onboarding-character-eyes-quick">
        <g transform="translate(94 109)">
          {QUICK_WITTED_EYES_PATHS.map((path) => (
            <path key={path} d={path} />
          ))}
        </g>
      </g>
    );
  }

  if (mood === "strategic") {
    return (
      <g className="onboarding-character-eyes onboarding-character-eyes-strategic">
        <g transform="translate(80.14 96.37) scale(0.78)">
          {STRATEGIC_EYES_PATHS.map((path) => (
            <path key={path} d={path} />
          ))}
          <g transform="translate(62 0) scale(0.86 1) translate(-62 0)">
            {STRATEGIC_EYEBROW_PATHS.map((path) => (
              <path key={path} d={path} />
            ))}
          </g>
        </g>
      </g>
    );
  }

  return (
    <g className="onboarding-character-eyes onboarding-character-eyes-clearheaded">
      <path fillRule="evenodd" clipRule="evenodd" d={CLOVY_EYES_PATH} />
    </g>
  );
}
