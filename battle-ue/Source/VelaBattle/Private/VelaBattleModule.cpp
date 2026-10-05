#include "VelaBattle.h"

#include "HAL/PlatformProcess.h"
#include "Misc/Paths.h"
#include "Modules/ModuleManager.h"

DEFINE_LOG_CATEGORY(LogVelaBattle);

namespace
{
	bool GCoreLoaded = false;
}

bool VelaBattleCoreLoaded()
{
	return GCoreLoaded;
}

class FVelaBattleModule : public FDefaultGameModuleImpl
{
public:
	virtual void StartupModule() override
	{
#if PLATFORM_WINDOWS
		// battlecore.dll is delay-loaded; load it here from a known place so a
		// missing DLL becomes a message on screen instead of a crash.
		const FString Name = TEXT("battlecore.dll");
		const TArray<FString> Candidates = {
			FPaths::Combine(FPaths::ProjectDir(), TEXT("Binaries/Win64"), Name),
			FPaths::Combine(FPaths::ProjectDir(), TEXT("Source/ThirdParty/BattleCore/lib/Win64"), Name),
			FPaths::Combine(FPlatformProcess::BaseDir(), Name),
		};
		for (const FString& Path : Candidates)
		{
			if (FPaths::FileExists(Path))
			{
				Handle = FPlatformProcess::GetDllHandle(*Path);
				if (Handle)
				{
					UE_LOG(LogVelaBattle, Log, TEXT("Loaded battle core from %s"), *Path);
					break;
				}
			}
		}
		GCoreLoaded = Handle != nullptr;
		if (!GCoreLoaded)
		{
			UE_LOG(LogVelaBattle, Error, TEXT("battlecore.dll not found. Run build_core.ps1 first."));
		}
#else
		GCoreLoaded = true; // linked directly
#endif
	}

	virtual void ShutdownModule() override
	{
		// The DLL stays loaded until the process exits: a battle that the
		// garbage collector frees late may still call bc_destroy.
		GCoreLoaded = false;
	}

private:
	void* Handle = nullptr;
};

IMPLEMENT_PRIMARY_GAME_MODULE(FVelaBattleModule, VelaBattle, "VelaBattle");
