#pragma once

#include "CoreMinimal.h"

DECLARE_LOG_CATEGORY_EXTERN(LogVelaBattle, Log, All);

// True once battlecore.dll is loaded. Nothing may call into the core before.
VELABATTLE_API bool VelaBattleCoreLoaded();
