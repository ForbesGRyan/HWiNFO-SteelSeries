using System;
using System.Collections.Generic;
using System.IO.MemoryMappedFiles;
using System.Runtime.InteropServices;

namespace SensorMonHTTP 
{
    class HWiNFOWrapper
    {
        public const string HWiNFO_SENSORS_MAP_FILE_NAME2 = "Global\\HWiNFO_SENS_SM2";
        public const string HWiNFO_SENSORS_SM2_MUTEX = "Global\\HWiNFO_SM2_MUTEX";
        public const int HWiNFO_SENSORS_STRING_LEN2 = 128;
        public const int HWiNFO_UNIT_STRING_LEN = 16;

        public enum SENSOR_READING_TYPE
        {
            SENSOR_TYPE_NONE = 0,
            SENSOR_TYPE_TEMP,
            SENSOR_TYPE_VOLT,
            SENSOR_TYPE_FAN,
            SENSOR_TYPE_CURRENT,
            SENSOR_TYPE_POWER,
            SENSOR_TYPE_CLOCK,
            SENSOR_TYPE_USAGE,
            SENSOR_TYPE_OTHER
        };

  
        [StructLayout(LayoutKind.Sequential, Pack = 1, CharSet = CharSet.Ansi)]
        public struct _HWiNFO_SENSORS_READING_ELEMENT
        {
            public SENSOR_READING_TYPE tReading;
            public UInt32 dwSensorIndex;
            public UInt32 dwReadingID;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = HWiNFO_SENSORS_STRING_LEN2)]
            public string szLabelOrig;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = HWiNFO_SENSORS_STRING_LEN2)]
            public string szLabelUser;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = HWiNFO_UNIT_STRING_LEN)]
            public string szUnit;
            public double Value;
            public double ValueMin;
            public double ValueMax;
            public double ValueAvg;
        };

        [StructLayout(LayoutKind.Sequential, Pack = 1, CharSet = CharSet.Ansi)]
        public struct _HWiNFO_SENSORS_SENSOR_ELEMENT
        {
           public UInt32 dwSensorID;
           public UInt32 dwSensorInst;
           [MarshalAs(UnmanagedType.ByValTStr, SizeConst = HWiNFO_SENSORS_STRING_LEN2)]
            public string szSensorNameOrig;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = HWiNFO_SENSORS_STRING_LEN2)]
            public string szSensorNameUser;
        };

        [StructLayout(LayoutKind.Sequential, Pack = 1, CharSet = CharSet.Ansi)]
        public struct _HWiNFO_SENSORS_SHARED_MEM2
        {
            public UInt32 dwSignature;
            public UInt32 dwVersion;
            public UInt32 dwRevision;
            public long poll_time;
            public UInt32 dwOffsetOfSensorSection;
            public UInt32 dwSizeOfSensorElement;
            public UInt32 dwNumSensorElements;
            // descriptors for the Readings section
            public UInt32 dwOffsetOfReadingSection; // Offset of the Reading section from beginning of HWiNFO_SENSORS_SHARED_MEM2
            public UInt32 dwSizeOfReadingElement;   // Size of each Reading element = sizeof( HWiNFO_SENSORS_READING_ELEMENT )
            public UInt32 dwNumReadingElements;     // Number of Reading elements
        };
    

	    MemoryMappedFile mmf;
        public uint numSensors { get; private set; }
        public uint numReadingElements { get; private set; }
        public List<string> masterSensorNames { get; private set; }
        public List<string> masterLabelUser { get; private set; }
        public Dictionary<string, Dictionary<string, List<string>>> Readings { get; private set; }

        private uint offsetSensorSection;
        private uint sizeSensorElement;
        private uint offsetReadingSection;
        private uint sizeReadingSection;

        // ***************************************************************************************************************
        //                                                  Code Example
        // ***************************************************************************************************************

        public void Open()
        {
            masterSensorNames = new List<string>();
            masterLabelUser = new List<string>();
            Readings = new Dictionary<string, Dictionary<string, List<string>>>();
            try
            {
                mmf = MemoryMappedFile.OpenExisting(HWiNFO_SENSORS_MAP_FILE_NAME2, MemoryMappedFileRights.Read);
            }
		    catch (Exception Ex)
            {
                Console.WriteLine("An error occured while opening the HWiNFO shared memory! - " + Ex.Message);
                Console.WriteLine("Press ENTER to exit program...");
                Console.ReadLine();
                Environment.Exit(1);
            }
            using (var accessor = mmf.CreateViewAccessor(0, Marshal.SizeOf(typeof(_HWiNFO_SENSORS_SHARED_MEM2)), MemoryMappedFileAccess.Read))
            {
			    _HWiNFO_SENSORS_SHARED_MEM2 HWiNFOMemory ;
                accessor.Read(0, out HWiNFOMemory);
			    numSensors = HWiNFOMemory.dwNumSensorElements ;
                numReadingElements = HWiNFOMemory.dwNumReadingElements;
			    offsetSensorSection = HWiNFOMemory.dwOffsetOfSensorSection ;
                sizeSensorElement = HWiNFOMemory.dwSizeOfSensorElement;
                offsetReadingSection = HWiNFOMemory.dwOffsetOfReadingSection;
                sizeReadingSection = HWiNFOMemory.dwSizeOfReadingElement;

			    for (UInt32 dwSensor = 0; dwSensor < numSensors; dwSensor ++)
                {
                    using (var sensor_element_accessor = mmf.CreateViewStream(offsetSensorSection + (dwSensor * sizeSensorElement), sizeSensorElement, MemoryMappedFileAccess.Read))
                    {
                    
                        byte[] byteBuffer = new byte[sizeSensorElement];
                        sensor_element_accessor.Read(byteBuffer,0,(int)sizeSensorElement);
                        GCHandle handle = GCHandle.Alloc(byteBuffer, GCHandleType.Pinned);
                        _HWiNFO_SENSORS_SENSOR_ELEMENT SensorElement = (_HWiNFO_SENSORS_SENSOR_ELEMENT)Marshal.PtrToStructure(handle.AddrOfPinnedObject(),
                            typeof(_HWiNFO_SENSORS_SENSOR_ELEMENT));
                        handle.Free();

                        masterSensorNames.Add(SensorElement.szSensorNameUser);
                        Readings.Add(SensorElement.szSensorNameUser, new Dictionary<string, List<string>>());
                    
					    //Console.WriteLine(String.Format("dwSensorID : {0}", SensorElement.dwSensorID));
					    //Console.WriteLine(String.Format("dwSensorInst : {0}", SensorElement.dwSensorInst));
					    //Console.WriteLine(String.Format("szSensorNameOrig : {0}", SensorElement.szSensorNameOrig));
					    //Console.WriteLine(String.Format("szSensorNameUser : {0}", SensorElement.szSensorNameUser));
                    }
                }
                for (UInt32 dwReading = 0; dwReading < numReadingElements; dwReading++)
                {
                    using (var sensor_element_accessor = mmf.CreateViewStream(offsetReadingSection + (dwReading * sizeReadingSection), sizeReadingSection, MemoryMappedFileAccess.Read))
                    {
                        byte[] byteBuffer = new byte[sizeReadingSection];
                        sensor_element_accessor.Read(byteBuffer,0,(int)sizeReadingSection);
                        GCHandle handle = GCHandle.Alloc(byteBuffer, GCHandleType.Pinned);
                        _HWiNFO_SENSORS_READING_ELEMENT ReadingElement = (_HWiNFO_SENSORS_READING_ELEMENT)Marshal.PtrToStructure(handle.AddrOfPinnedObject(),
                            typeof(_HWiNFO_SENSORS_READING_ELEMENT));
                        handle.Free();
                        masterLabelUser.Add(ReadingElement.szLabelUser);
                        List<string> values = new List<string>();
                        values.Add(ReadingElement.szUnit);
                        values.Add(ReadingElement.Value.ToString());
                        values.Add(ReadingElement.ValueMin.ToString());
                        values.Add(ReadingElement.ValueMax.ToString());
                        values.Add(ReadingElement.ValueAvg.ToString());
                        string key = String.Format("{0} {1}", masterSensorNames[(int)ReadingElement.dwSensorIndex], ReadingElement.szLabelUser);
                        try
                        {
                            //Readings.Add(key, values);
                            Readings[masterSensorNames[(int)ReadingElement.dwSensorIndex]].Add(ReadingElement.szLabelUser, values);
                        }
                        catch (Exception e)
                        {
                            
                        }
                        //Console.WriteLine(String.Format("tReading : {0}", ReadingElement.tReading));
                        //Console.WriteLine(String.Format("dwSensorIndex : {0} ; Sensor Name: {1}", ReadingElement.dwSensorIndex, masterSensorNames[(int)ReadingElement.dwSensorIndex]));
                        //Console.WriteLine(String.Format("dwReadingID : {0}", ReadingElement.dwSensorIndex));
                        //Console.WriteLine(String.Format("szLabelUser : {0}", ReadingElement.szLabelUser));
                        //Console.WriteLine(String.Format("szUnit : {0}", ReadingElement.szUnit));
                        //Console.WriteLine(String.Format("Value : {0}", ReadingElement.Value));
                    }
                }

            }
 	    }
	    public void Close()
        {
            if (mmf != null)
            {
                mmf.Dispose();
            }
        }
        
    };

}

// ***************************************************************************************************************
//                                          HWiNFO Shared Memory Footprint
// ***************************************************************************************************************
//
//         |-----------------------------|-----------------------------------|-----------------------------------|
// Content |  HWiNFO_SENSORS_SHARED_MEM2 |  HWiNFO_SENSORS_SENSOR_ELEMENT[]  | HWiNFO_SENSORS_READING_ELEMENT[]  |
//         |-----------------------------|-----------------------------------|-----------------------------------|
// Pointer |<--0                         |<--dwOffsetOfSensorSection         |<--dwOffsetOfReadingSection        |
//         |-----------------------------|-----------------------------------|-----------------------------------|
// Size    |  dwOffsetOfSensorSection    |   dwSizeOfSensorElement           |    dwSizeOfReadingElement         |
//         |                             |      * dwNumSensorElement         |       * dwNumReadingElement       |
//         |-----------------------------|-----------------------------------|-----------------------------------|
//
