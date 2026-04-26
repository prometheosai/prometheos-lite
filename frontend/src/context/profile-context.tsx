"use client"

import { createContext, useContext, useState, useEffect, ReactNode } from "react"

interface Profile {
  name: string
  email: string
  initials: string
  avatarUrl: string | null
}

interface ProfileContextType {
  profile: Profile
  updateProfile: (profile: Profile) => void
}

const ProfileContext = createContext<ProfileContextType | undefined>(undefined)

export function ProfileProvider({ children }: { children: ReactNode }) {
  const [profile, setProfile] = useState<Profile>({
    name: "Diego Rhoger",
    email: "diego@example.com",
    initials: "DR",
    avatarUrl: null
  })

  // Load profile from localStorage on mount
  useEffect(() => {
    const savedProfile = localStorage.getItem("userProfile")
    if (savedProfile) {
      try {
        setProfile(JSON.parse(savedProfile))
      } catch (e) {
        console.error("Failed to parse saved profile:", e)
      }
    }
  }, [])

  const updateProfile = (newProfile: Profile) => {
    setProfile(newProfile)
    localStorage.setItem("userProfile", JSON.stringify(newProfile))
    // TODO: Save to database via API
    console.log("Saving profile to database:", newProfile)
  }

  return (
    <ProfileContext.Provider value={{ profile, updateProfile }}>
      {children}
    </ProfileContext.Provider>
  )
}

export function useProfile() {
  const context = useContext(ProfileContext)
  if (context === undefined) {
    throw new Error("useProfile must be used within a ProfileProvider")
  }
  return context
}
