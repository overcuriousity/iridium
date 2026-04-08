# Requirements Specification: Reimplementation of Guymager
**Project Goal:** Development of a GUI-based forensic software tool for creating immutable disk images (Images) with completeness and integrity assurance.

## 1. Introduction and Objectives
The goal is to create a tool that fulfills the forensic standards for **forensically sound creation of disk images**. Guymager serves as the interface (GUI) for the `libewf` library (specifically `ewfacquire`). The reimplementation must ensure that every backup is a **1:1 copy** of the disk, without altering the original data.

### 1.1 Core Requirements for Forensic Duplication
The software must implement the following four pillars [3][9]:
1.  **Physical Copy:** The entire sector content of all sectors (including "free" areas) must be copied.
2.  **Error Handling:** Read errors must be detected, logged, and replaced with predetermined fill patterns (e.g., zeros) without aborting the process [3].
3.  **Completeness:** Reserved areas such as Host Protected Area (HPA) and Device Configuration Overlay (DCO) must be detected, disabled, and backed up to ensure a complete image [3][9].
4.  **Immutability:** The creation process must be completed with the calculation of a cryptographic checksum (hash) to make integrity verifiable [3][9].

## 2. Functional Requirements

### 2.1 Disk Detection and Selection
*   **Function:** The tool must list all disks detected in the system.
*   **Requirement:** Display of device name, size, and type.
*   **Forensic Aspect:** Access to the source disk must be read-only. The software must ensure that no write operations are performed on the original [2].

### 2.2 Image Creation (Acquisition)
*   **Supported Formats:**
    *   **RAW (dd format):** Uncompressed bit-for-bit copy.
    *   **EWF (Expert Witness Format):** Compressed or uncompressed, supports multiple files.
    *   **AFF (Advanced Forensic Format):** Standard for forensic images.
*   **Functionality:**
    *   Creation of images in the selected format.
    *   Support for parallel reading and writing to utilize multiprocessor systems (Hyper-Threading) [1].
    *   Calculation of hashes (MD5, SHA256) during the write process and storage in the image file [1][5].

### 2.3 Error Handling and Recovery
*   **Logic:** The tool must not abort upon encountering read errors.
*   **Implementation:** Use of `noerror` options (similar to `dd_rescue` or `dc3dd`) to ignore read errors and continue reading [2].
*   **Synchronization:** Synchronization of input and output upon errors (`sync`) to ensure data consistency [2].
*   **Documentation:** All read errors must be logged in a separate log file.

### 2.4 Handling Hidden Areas (HPA/DCO)
*   **Function:** The tool must check if HPA or DCO are active.
*   **Procedure:** These areas must be securely detected and disabled at the time of image creation to obtain a complete copy [3].
*   **Imaging:** The backup must cover the entire physical size, even if the operating system does not see these areas.

### 2.5 Integrity Check (Verification)
*   **Automatic Checksum Comparison:** Upon completion of the backup, the hash value of the image must be compared with the calculated hash value of the source disk [1].
*   **Result:** Only upon matching is the image considered a valid piece of evidence.
*   **Storage:** Checksums must be stored in a separate file or within the image header [5].

### 2.6 User Interface (GUI)
*   **Progress Indicator:** Visual representation of progress (e.g., percentage display, speed, remaining time).
*   **Duplication Station Integration:** The software should be designed so that it can also be used on special hardware duplication stations (with write blockers) [2].

## 3. Technical Requirements and Architecture

### 3.1 Backend Library
*   The reimplementation must use **`libewf`** (Library for EWF), specifically the `ewfacquire` module. This guarantees the forensic correctness of the underlying algorithms [2][4].
*   Alternative/complementary use of `dcfldd` or `dd_rescue` for special recovery scenarios (cluster-based reading, data streams) [1].

### 3.2 Platform Support
*   **Operating System:** Linux (Open-Source focus), but potential porting to Windows/macOS for broader usability [1][4].
*   **Hardware Support:** Compatibility with common hardware write blockers (e.g., Tableau, etc.) to physically prevent write access [2].

### 3.3 Security and Privacy
*   **No Changes to Host:** The tool must not alter the operating system on which it runs (no installation of drivers or persistence).
*   **Writeblocker Integration:** When used without a hardware write blocker, the software must ensure that no write operations go to the source disk (e.g., by using loop devices or virtual interfaces) [2].
*   **Completely Offline** for operation in airgapped environments, compiled as a single rust binary

## 4. Non-Functional Requirements

### 4.1 Performance
*   **Parallelization:** Full utilization of multiprocessor systems for reading, hash calculation, and writing [1].
*   **Efficiency:** Minimization of write operations to the source disk through caching or direct block access.

### 4.2 Reliability and Repeatability
*   The methods used must be accepted in the professional community and yield the same results when applied by third parties [3].
*   Robustness with respect to defective disks (sector errors) is mandatory [3].

### 4.3 Documentation and Audit Trail
*   **Chain of Custody:** Every image must be provided with metadata: evidence number, operator, date/time, hash values [5].
*   **Logging:** Every step (start, error, completion) must be logged. Log every exact command with exact parameters, or any operation. log should be able to be exported, requires precise capturing of timestamps. should also support optional PGP signature of operator for integrity. Log version of any library or program at the time of operation (or compilation)

## 5. Specific Scenarios and Edge Cases

### 5.1 Defective Hard Drives (Recovery Mode)
*   The tool must offer a "Recovery" mode that works like `dd_rescue`: reading from back to front to save data before it is overwritten [1].
*   Support for data streams (streaming) for transferring damaged data [1].

### 5.2 RAID and Complex File Systems
*   Support for native interpretation of RAID systems (JBOD, Level 0, 5, 6, LVM2), if the tool goes beyond pure image creation or mounts images of these structures [1].
*   *Note:* The primary focus lies on the **bit-accurate backup** of the raw disk. The interpretation of RAID structures is the task of analysis tools (such as X-Ways or EnCase), but the image must be created in such a way that these structures can be reconstructed later [1].

### 5.3 Large Disks (> 2 TB)
*   Support for images larger than 2 TB (with more than $2^{32}$ sectors) and sector sizes up to 8 KB [1].

## 6. Success Criteria (Acceptance Criteria)

1.  **Hash Match:** A generated image must exactly match at every comparison of the hash value with the original.
2.  **Completeness:** The image must cover the entire physical size of the disk, including hidden areas (HPA/DCO).
3.  **Error Management:** For disks with read errors, the tool must generate a complete image (with zero fill patterns) without aborting, and document the errors in the log.
4.  **Forensic Admissibility:** The generated format (EWF/AFF) must be readable by standard tools such as EnCase or X-Ways and be able to confirm integrity [1][3].
5.  **User-Friendliness:** The GUI must offer simple operation to avoid misuse that could lead to data changes [2].

---

**Note on Implementation:**
The reimplementation should not attempt to replace `dd` or `libewf`, but rather use/ship them as core components. The added value of Guymager lies in the **abstraction** and **user-friendliness**. Forensic correctness derives directly from the correct use of `libewf/ewfacquire` [2][4].
