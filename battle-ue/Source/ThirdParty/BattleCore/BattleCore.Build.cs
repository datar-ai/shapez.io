using System.IO;
using UnrealBuildTool;

// The Rust battle core (../../../../battle-core) as a prebuilt DLL.
// build_core.ps1 builds it and copies battlecore.dll and its import library
// into lib/Win64 next to this file.
public class BattleCore : ModuleRules
{
	public BattleCore(ReadOnlyTargetRules Target) : base(Target)
	{
		Type = ModuleType.External;

		string CoreRoot = Path.GetFullPath(Path.Combine(ModuleDirectory, "..", "..", "..", "..", "battle-core"));
		PublicSystemIncludePaths.Add(Path.Combine(CoreRoot, "include"));

		if (Target.Platform == UnrealTargetPlatform.Win64)
		{
			string LibDir = Path.Combine(ModuleDirectory, "lib", "Win64");
			PublicAdditionalLibraries.Add(Path.Combine(LibDir, "battlecore.dll.lib"));
			// Loaded by FVelaBattleModule::StartupModule from a known path,
			// so the game can show an error instead of crashing when it is missing.
			PublicDelayLoadDLLs.Add("battlecore.dll");
			RuntimeDependencies.Add("$(TargetOutputDir)/battlecore.dll", Path.Combine(LibDir, "battlecore.dll"));
		}
		else if (Target.Platform == UnrealTargetPlatform.Linux)
		{
			string LibDir = Path.Combine(ModuleDirectory, "lib", "Linux");
			PublicAdditionalLibraries.Add(Path.Combine(LibDir, "libbattlecore.so"));
			RuntimeDependencies.Add("$(TargetOutputDir)/libbattlecore.so", Path.Combine(LibDir, "libbattlecore.so"));
		}
	}
}
